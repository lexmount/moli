use super::*;

#[test]
fn no_source_no_cache_mutations_do_not_queue_retained_invalidation_work() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let child = host.create_element("span");
    let target = host.create_element("div");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, child));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let epoch = engine.target_context_epoch_for_document_for_test(document);

    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent,
            added_nodes: vec![child],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: None,
            next_sibling: None,
        }],
        &media,
    );
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: target,
            namespace: None,
            name: "class".to_owned(),
            old_value: None,
            new_value: Some("active".to_owned()),
        }],
        &media,
    );

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.target_context_epoch_for_document_for_test(document),
        epoch + 2
    );
}

#[test]
fn mutation_source_scope_presence_uses_cheap_effect_classification() {
    let mut host = test_host();
    let document = host.document_handle();
    let connected = host.create_element("section");
    let detached = host.create_element("article");
    assert!(host.append_child(document, connected));

    let cases = [
        (
            Vec::new(),
            false,
            "empty effects should not have a source scope",
        ),
        (
            vec![StyleMutationEffect::DisconnectedSubtrees {
                roots: vec![detached].into(),
            }],
            false,
            "only disconnected subtree effects should not have a source scope",
        ),
        (
            vec![StyleMutationEffect::ConnectedSubtrees {
                roots: vec![connected].into(),
            }],
            true,
            "connected subtree effects should have a source scope",
        ),
        (
            vec![StyleMutationEffect::ChildList {
                parent: document,
                added_nodes: vec![connected],
                removed_nodes: Vec::new(),
                removed_element_snapshots: Vec::new(),
                previous_sibling: None,
                next_sibling: None,
            }],
            true,
            "child-list effects should have a source scope",
        ),
    ];

    for (effects, expected, message) in cases {
        assert_eq!(
            mutation_effects_have_source_scope(&effects),
            expected,
            "{message}"
        );
        assert_eq!(
            source_scope_for_mutations(&host, &effects).is_some(),
            expected,
            "{message}"
        );
    }
}

#[test]
fn retained_system_mutations_without_computed_cache_still_queue_work() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let parent = host.create_element("section");
    let child = host.create_element("span");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, parent));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "section:has(span) { color: red; }".into(),
    );
    let inputs = FullStyleWorldSnapshot::default();
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, host.document_handle(), key, &inputs);
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    let epoch = engine.target_context_epoch_for_document_for_test(document);

    assert!(host.append_child(parent, child));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent,
            added_nodes: vec![child],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: None,
            next_sibling: None,
        }],
        &media,
    );

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.target_context_epoch_for_document_for_test(document),
        epoch + 1
    );
}

#[test]
fn has_selector_child_list_invalidation_uses_target_queries_without_rebuilding_stylist() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let style_text = host.create_text_node("body:has(span) .subject { color: red; }");
    let body = host.create_element("body");
    let container = host.create_element("section");
    let outside = host.create_element("div");
    let subject = host.create_element("div");
    assert!(host.set_attribute(subject, "class", "subject"));

    assert!(host.append_child(style, style_text));
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, outside));
    assert!(host.append_child(body, container));
    assert!(host.append_child(body, subject));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "body:has(span) .subject { color: red; }".into(),
    );
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
                subject,
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

    let added = host.create_element("span");
    assert!(host.append_child(container, added));
    let effects = [StyleMutationEffect::ChildList {
        parent: container,
        added_nodes: vec![added],
        removed_nodes: Vec::new(),
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let source_scope = style_source_scope_for_mutation_effects(&host, &effects);
    assert!(
        engine.test_author_sources_have_relative_selector_dependency_for_document(
            &host,
            document,
            &source_scope,
            &media,
        )
    );
    let planned_scope = source_scope_for_mutations(&host, &effects);
    assert_eq!(planned_scope, Some(source_scope));

    engine.invalidate_for_mutations(&host, &effects, &media);

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "the broad :has() cleanup must retain published descendants until observation"
    );
    assert!(
        engine.retained_style_invalidation_root_count_for_document_for_test(document) > 0,
        "the broad target-query result must be retained as lazy roots"
    );
}

#[test]
fn has_selector_child_list_invalidation_collects_inserted_subtree_dependency_keys() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let style_text = host.create_text_node(".subject:has(.descendant) { color: rgb(1, 2, 3); }");
    let body = host.create_element("body");
    let subject = host.create_element("div");
    assert!(host.set_attribute(subject, "class", "subject"));

    assert!(host.append_child(style, style_text));
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, subject));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".subject:has(.descendant) { color: rgb(1, 2, 3); }".into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    let initial = engine
        .computed_style_property_value(&host, &document_url, subject, "color", None, &inputs, None)
        .expect("subject color should compute before insertion");
    assert_ne!(initial, "rgb(1, 2, 3)");
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let wrapper = host.create_element("div");
    let descendant = host.create_element("div");
    assert!(host.set_attribute(descendant, "class", "descendant"));
    assert!(host.append_child(wrapper, descendant));
    assert!(host.append_child(subject, wrapper));
    let effects = [StyleMutationEffect::ChildList {
        parent: subject,
        added_nodes: vec![wrapper],
        removed_nodes: Vec::new(),
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0,
        "the inserted root itself has no .descendant dependency key; retained invalidation must collect keys from the inserted subtree"
    );
}

#[test]
fn has_selector_child_list_invalidation_matches_inserted_heading_pseudo_class() {
    let mut host = test_host();
    host.reset_html_document_shell();
    let document = host.document_handle();
    let body = host.document_body_handle().unwrap();
    let ancestor = host.create_element("section");
    let subject = host.create_element("div");
    assert!(host.set_attribute(ancestor, "id", "ancestor"));
    assert!(host.set_attribute(subject, "id", "subject"));

    assert!(host.append_child(body, ancestor));
    assert!(host.append_child(body, subject));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "#ancestor:has(:heading(1)) ~ #subject { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    let initial = engine
        .computed_style_property_value(&host, &document_url, subject, "color", None, &inputs, None)
        .expect("subject color should compute before insertion");
    assert_ne!(initial, "rgb(1, 2, 3)");
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let heading = host.create_element("h1");
    assert!(host.append_child(ancestor, heading));
    let effects = [StyleMutationEffect::ChildList {
        parent: ancestor,
        added_nodes: vec![heading],
        removed_nodes: Vec::new(),
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
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
        Some("rgb(1, 2, 3)".into())
    );
}

#[test]
fn has_selector_child_list_invalidation_matches_inserted_adjacent_heading_pseudo_class() {
    let mut host = test_host();
    host.reset_html_document_shell();
    let document = host.document_handle();
    let body = host.document_body_handle().unwrap();
    let style_text = "#target { color: rgb(128, 128, 128); }
         #sibling:has(+ :heading(1)) ~ #target { color: rgb(1, 2, 3); }";
    let sibling = host.create_element("div");
    let target = host.create_element("div");
    let unrelated = host.create_element("div");
    assert!(host.set_attribute(sibling, "id", "sibling"));
    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.set_attribute(unrelated, "id", "unrelated"));

    assert!(host.append_child(body, sibling));
    assert!(host.append_child(body, target));
    assert!(host.append_child(body, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(style_text.into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    let initial = engine
        .computed_style_property_value(&host, &document_url, target, "color", None, &inputs, None)
        .expect("target color should compute before insertion");
    assert_eq!(initial, "rgb(128, 128, 128)");
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                unrelated,
                "display",
                None,
                &inputs,
                None
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let heading = host.create_element("h1");
    assert!(host.insert_before(body, heading, Some(target)));
    let effects = [StyleMutationEffect::ChildList {
        parent: body,
        added_nodes: vec![heading],
        removed_nodes: Vec::new(),
        removed_element_snapshots: Vec::new(),
        previous_sibling: Some(sibling),
        next_sibling: Some(target),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, target),
        "an inserted :heading() can satisfy a previous-sibling :has(+ ...) selector whose subject then affects later siblings"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &inputs,
            None
        ),
        Some("rgb(1, 2, 3)".into())
    );
}

#[test]
fn has_selector_child_list_insertion_ignores_unrelated_element_type() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let style_text = host.create_text_node("body:has(span) .subject { color: red; }");
    let body = host.create_element("body");
    let container = host.create_element("section");
    let outside = host.create_element("div");
    let subject = host.create_element("div");
    let inserted = host.create_element("p");
    assert!(host.set_attribute(subject, "class", "subject"));

    assert!(host.append_child(style, style_text));
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, outside));
    assert!(host.append_child(body, container));
    assert!(host.append_child(body, subject));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "body:has(span) .subject { color: red; }".into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [outside, subject] {
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

    assert!(host.append_child(container, inserted));
    let effects = [StyleMutationEffect::ChildList {
        parent: container,
        added_nodes: vec![inserted],
        removed_nodes: Vec::new(),
        removed_element_snapshots: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
}

#[test]
fn stylo_tree_invalidator_wrapper_collects_sibling_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let marker = host.create_element("div");
    let target = host.create_element("p");
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, marker));
    assert!(host.append_child(document, target));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ".marker + .target { color: red; }".into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        ))),
    );
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, host.document_handle(), key, &inputs);

    let (roots, requires_fallback) = collect_source_invalidation_roots_for_test(
        &engine,
        &host,
        marker,
        StyloStyleInvalidationQuery::Class("marker"),
    );

    assert!(!requires_fallback);
    assert!(roots.contains(&target));
    assert!(!roots.contains(&document));
}

#[test]
fn stylo_tree_invalidator_wrapper_handles_normal_dependency_chains() {
    let mut host = test_host();
    let document = host.document_handle();
    let scope = host.create_element("section");
    let marker = host.create_element("div");
    let target = host.create_element("p");
    assert!(host.set_attribute(scope, "class", "scope"));
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, scope));
    assert!(host.append_child(scope, marker));
    assert!(host.append_child(scope, target));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ".scope :is(.marker) + .target { color: red; }".into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        ))),
    );
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, host.document_handle(), key, &inputs);

    let (roots, requires_fallback) = collect_source_invalidation_roots_for_test(
        &engine,
        &host,
        marker,
        StyloStyleInvalidationQuery::Class("marker"),
    );

    assert!(!requires_fallback);
    assert!(roots.contains(&target));
    assert!(!roots.contains(&document));
}

#[test]
fn stylo_tree_invalidator_wrapper_handles_scope_dependency_chains() {
    let mut host = test_host();
    let document = host.document_handle();
    let scope = host.create_element("section");
    let marker = host.create_element("div");
    let target = host.create_element("p");
    let unrelated = host.create_element("p");
    assert!(host.set_attribute(scope, "class", "scope"));
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, scope));
    assert!(host.append_child(scope, marker));
    assert!(host.append_child(scope, target));
    assert!(host.append_child(document, unrelated));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ":scope .marker + .target { color: red; }".into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        ))),
    );
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, host.document_handle(), key, &inputs);

    let (roots, requires_fallback) = collect_source_invalidation_roots_for_test(
        &engine,
        &host,
        marker,
        StyloStyleInvalidationQuery::Class("marker"),
    );

    assert!(!requires_fallback, "roots={roots:?}");
    assert!(roots.contains(&target), "roots={roots:?}");
    assert!(!roots.contains(&unrelated), "roots={roots:?}");
    assert!(!roots.contains(&document), "roots={roots:?}");
}

#[test]
fn stylo_tree_invalidator_wrapper_accepts_empty_scope_dependency_result() {
    let mut host = test_host();
    let document = host.document_handle();
    let marker = host.create_element("div");
    let spacer = host.create_element("p");
    let target = host.create_element("p");
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, marker));
    assert!(host.append_child(document, spacer));
    assert!(host.append_child(document, target));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ":scope .marker + .target { color: red; }".into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 0,
        ))),
    );
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, host.document_handle(), key, &inputs);

    let (roots, requires_fallback) = collect_source_invalidation_roots_for_test(
        &engine,
        &host,
        marker,
        StyloStyleInvalidationQuery::Class("marker"),
    );

    assert!(!requires_fallback);
    assert!(roots.is_empty(), "{roots:?}");
}

#[test]
fn stylo_style_adapter_persists_selector_flags() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let child = host.create_element("div");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, child));

    let adapter = StyloDomStyleAdapter::new();
    adapter.with_bound_host(&host, |binding| {
        let element = binding.element(&host, child).unwrap();
        assert!(!element.has_selector_flags(ElementSelectorFlags::ANCHORS_RELATIVE_SELECTOR));
        assert!(element.relative_selector_search_direction().is_empty());

        element.apply_selector_flags(
            ElementSelectorFlags::ANCHORS_RELATIVE_SELECTOR
                | ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING
                | ElementSelectorFlags::HAS_EDGE_CHILD_SELECTOR,
        );

        assert!(element.has_selector_flags(ElementSelectorFlags::ANCHORS_RELATIVE_SELECTOR));
        assert!(
            element
                .relative_selector_search_direction()
                .contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING)
        );

        let parent_element = binding.element(&host, parent).unwrap();
        assert!(parent_element.has_selector_flags(ElementSelectorFlags::HAS_EDGE_CHILD_SELECTOR));
    });

    adapter.with_bound_host(&host, |binding| {
        let element = binding.element(&host, child).unwrap();
        assert!(element.has_selector_flags(ElementSelectorFlags::ANCHORS_RELATIVE_SELECTOR));
        assert!(
            element
                .relative_selector_search_direction()
                .contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING)
        );
    });
}

#[test]
fn stylo_style_resolution_records_relative_selector_flags() {
    let mut host = test_host();
    let document = host.document_handle();
    let anchor = host.create_element("section");
    let marker = host.create_element("span");
    assert!(host.set_attribute(anchor, "class", "anchor"));
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.append_child(document, anchor));
    assert!(host.append_child(anchor, marker));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".anchor:has(.marker) { color: red; }".into(),
        document_url.clone(),
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                anchor,
                "color",
                None,
                &inputs,
                None,
            )
            .is_some()
    );

    engine.dom_adapter.with_bound_host(&host, |binding| {
        let anchor_element = binding.element(&host, anchor).unwrap();
        assert!(anchor_element.has_selector_flags(ElementSelectorFlags::ANCHORS_RELATIVE_SELECTOR));

        let marker_element = binding.element(&host, marker).unwrap();
        assert!(
            marker_element
                .relative_selector_search_direction()
                .contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR)
        );
    });
}

#[test]
fn detached_subtree_invalidation_clears_relative_selector_flags() {
    let mut host = test_host();
    let document = host.document_handle();
    let anchor = host.create_element("section");
    let marker = host.create_element("span");
    assert!(host.set_attribute(anchor, "class", "anchor"));
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.append_child(document, anchor));
    assert!(host.append_child(anchor, marker));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".anchor:has(.marker) { color: red; }".into(),
        document_url.clone(),
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                anchor,
                "color",
                None,
                &inputs,
                None,
            )
            .is_some()
    );

    engine.dom_adapter.with_bound_host(&host, |binding| {
        let anchor_element = binding.element(&host, anchor).unwrap();
        assert!(anchor_element.has_selector_flags(ElementSelectorFlags::ANCHORS_RELATIVE_SELECTOR));
        let marker_element = binding.element(&host, marker).unwrap();
        assert!(
            marker_element
                .relative_selector_search_direction()
                .contains(ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR)
        );
    });

    assert!(host.remove_child(document, anchor));
    engine.invalidate_detached_style_subtrees_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: document,
            added_nodes: Vec::new(),
            removed_nodes: vec![anchor],
            removed_element_snapshots: Vec::new(),
            previous_sibling: None,
            next_sibling: None,
        }],
    );

    engine.dom_adapter.with_bound_host(&host, |binding| {
        let anchor_element = binding.element(&host, anchor).unwrap();
        assert!(
            !anchor_element.has_selector_flags(ElementSelectorFlags::ANCHORS_RELATIVE_SELECTOR)
        );
        let marker_element = binding.element(&host, marker).unwrap();
        assert!(
            marker_element
                .relative_selector_search_direction()
                .is_empty()
        );
    });
}
