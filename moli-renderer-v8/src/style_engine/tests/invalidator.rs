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
            vec![StyleMutationEffect::DisconnectedSubtree { root: detached }],
            false,
            "only disconnected subtree effects should not have a source scope",
        ),
        (
            vec![StyleMutationEffect::ConnectedSubtree { root: connected }],
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
        0
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
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, parent));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, child));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
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
fn retained_stylo_invalidator_clears_shadow_host_for_shadow_tree_has_dependency() {
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
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_host)
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
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
fn retained_stylo_invalidator_clears_shadow_subject_for_host_context_inside_has_dependency() {
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
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
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
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_host)
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
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
#[test]
fn retained_stylo_invalidator_narrows_focus_sibling_cache_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("span");
    let source = host.create_element("button");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(source, "class", "focusable"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        ".focusable:focus + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    for handle in [previous, target, unrelated] {
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
    engine.invalidate_for_focus_change(&host, None, Some(source), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn retained_stylo_invalidator_narrows_checked_sibling_cache_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("span");
    let source = host.create_element("input");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(source, "class", "toggle"));
    assert!(host.set_attribute(source, "type", "checkbox"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        ".toggle:checked + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    for handle in [previous, target, unrelated] {
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
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        source,
        StyloElementState::CHECKED,
        None,
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_clears_defined_state_sibling_cache() {
    let mut host = test_host();
    let document = host.document_handle();
    let source = host.create_element("elucidate-late");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(source, "id", "source"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        "#source:defined + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    for handle in [source, target, unrelated] {
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

    let old_state = engine
        .retained_current_element_state(&host, source)
        .expect("retained state should be available after computed style read");
    assert!(host.set_custom_element_state(source, crate::dom::native::CustomElementState::Custom,));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        source,
        StyloElementState::DEFINED,
        Some(old_state),
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, source));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_clears_has_defined_ancestor_subject_cache() {
    let mut host = test_host();
    let document = host.document_handle();
    let subject = host.create_element("section");
    let source = host.create_element("my-element");
    let unrelated = host.create_element("section");

    assert!(host.set_attribute(subject, "id", "subject"));
    assert!(host.set_attribute(source, "id", "source"));
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.append_child(document, subject));
    assert!(host.append_child(subject, source));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        "#subject:has(:defined) { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    for handle in [subject, source, unrelated] {
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

    let old_state = engine
        .retained_current_element_state(&host, source)
        .expect("retained state should be available after computed style read");
    assert!(host.set_custom_element_state(source, crate::dom::native::CustomElementState::Custom,));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        source,
        StyloElementState::DEFINED,
        Some(old_state),
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
}
#[test]
fn retained_stylo_invalidator_uses_old_state_snapshots_for_radio_peer_batch() {
    let mut host = test_host();
    let document = host.document_handle();
    let first = host.create_element("input");
    let first_target = host.create_element("span");
    let second = host.create_element("input");
    let second_target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(first, "id", "first"));
    assert!(host.set_attribute(first, "type", "radio"));
    assert!(host.set_attribute(first, "name", "group"));
    assert!(host.set_attribute(first_target, "id", "firstTarget"));
    assert!(host.set_attribute(second, "id", "second"));
    assert!(host.set_attribute(second, "type", "radio"));
    assert!(host.set_attribute(second, "name", "group"));
    assert!(host.set_attribute(second_target, "id", "secondTarget"));
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.set_checked_state(first, true));
    assert!(host.append_child(document, first));
    assert!(host.append_child(document, first_target));
    assert!(host.append_child(document, second));
    assert!(host.append_child(document, second_target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        "#first:checked + #firstTarget { color: red; }
         #second:checked + #secondTarget { color: blue; }"
            .into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    for handle in [first_target, second_target, unrelated] {
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

    let first_old_state = engine
        .retained_current_element_state(&host, first)
        .expect("first radio retained state should be available");
    let second_old_state = engine
        .retained_current_element_state(&host, second)
        .expect("second radio retained state should be available");
    assert!(host.set_checked_state(second, true));

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    for (source, old_state) in [(first, first_old_state), (second, second_old_state)] {
        engine.invalidate_for_element_state_change_with_old_state(
            &host,
            source,
            StyloElementState::CHECKED
                | StyloElementState::INDETERMINATE
                | StyloElementState::VALIDITY_STATES,
            Some(old_state),
            &media,
        );
    }
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, first_target)
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, second_target)
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn retained_stylo_invalidator_uses_old_state_snapshot_for_range_sibling_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("span");
    let source = host.create_element("input");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(source, "id", "range"));
    assert!(host.set_attribute(source, "type", "number"));
    assert!(host.set_attribute(source, "min", "0"));
    assert!(host.set_attribute(source, "max", "10"));
    assert!(host.set_attribute(source, "value", "5"));
    assert!(host.set_input_value(source, "5"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        "#range:out-of-range + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    for handle in [previous, target, unrelated] {
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

    let old_state = engine
        .retained_current_element_state(&host, source)
        .expect("retained state should be available after computed style read");
    assert!(host.set_input_value(source, "20"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        source,
        StyloElementState::INRANGE
            | StyloElementState::OUTOFRANGE
            | StyloElementState::VALIDITY_STATES,
        Some(old_state),
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_treats_readonly_as_range_state_change() {
    let mut host = test_host();
    let document = host.document_handle();
    let source = host.create_element("input");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(source, "id", "range"));
    assert!(host.set_attribute(source, "type", "number"));
    assert!(host.set_attribute(source, "min", "0"));
    assert!(host.set_attribute(source, "max", "10"));
    assert!(host.set_attribute(source, "value", "5"));
    assert!(host.set_input_value(source, "5"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        "#range:in-range + .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
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

    let old_state = engine
        .retained_current_element_state(&host, source)
        .expect("retained state should be available after computed style read");
    assert!(host.set_attribute(source, "readonly", ""));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        source,
        StyloElementState::READONLY
            | StyloElementState::READWRITE
            | StyloElementState::INRANGE
            | StyloElementState::OUTOFRANGE
            | StyloElementState::VALIDITY_STATES,
        Some(old_state),
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_treats_readonly_as_validity_candidate_change() {
    let mut host = test_host();
    let document = host.document_handle();
    let subject = host.create_element("section");
    let input = host.create_element("input");

    assert!(host.set_attribute(subject, "id", "subject"));
    assert!(host.set_attribute(input, "id", "textinput"));
    assert!(host.set_attribute(input, "type", "text"));
    assert!(host.set_attribute(input, "required", ""));
    assert!(host.append_child(document, subject));
    assert!(host.append_child(subject, input));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        "#subject:has(#textinput:read-only) { color: rgb(135, 206, 235); }
         #subject:has(#textinput:valid) { color: rgb(144, 238, 144); }"
            .into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            subject,
            "color",
            None,
            &inputs,
            None
        ),
        Some("rgb(0, 0, 0)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let old_state = engine
        .retained_current_element_state(&host, input)
        .expect("retained state should be available after computed style read");
    assert!(host.set_attribute(input, "readonly", ""));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        input,
        StyloElementState::READONLY
            | StyloElementState::READWRITE
            | StyloElementState::INRANGE
            | StyloElementState::OUTOFRANGE
            | StyloElementState::VALIDITY_STATES,
        Some(old_state),
        &media,
    );
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
            None
        ),
        Some("rgb(135, 206, 235)".into())
    );
}

#[test]
fn value_state_change_keeps_unrelated_sibling_cache_entries() {
    let mut host = test_host();
    let document = host.document_handle();
    let source = host.create_element("input");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(source, "id", "range"));
    assert!(host.set_attribute(source, "type", "number"));
    assert!(host.set_attribute(source, "min", "0"));
    assert!(host.set_attribute(source, "max", "10"));
    assert!(host.set_attribute(source, "value", "5"));
    assert!(host.set_input_value(source, "5"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        ".unrelated + .target { color: blue; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
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

    let old_state = engine
        .retained_current_element_state(&host, source)
        .expect("retained state should be available after computed style read");
    assert!(host.set_input_value(source, "20"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        source,
        StyloElementState::INRANGE
            | StyloElementState::OUTOFRANGE
            | StyloElementState::VALIDITY_STATES,
        Some(old_state),
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn retained_stylo_invalidator_uses_state_snapshots_for_range_batches() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_source = host.create_element("input");
    let first_target = host.create_element("span");
    let second_source = host.create_element("input");
    let second_target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(first_source, "id", "first-range"));
    assert!(host.set_attribute(first_source, "type", "number"));
    assert!(host.set_attribute(first_source, "min", "0"));
    assert!(host.set_attribute(first_source, "max", "10"));
    assert!(host.set_attribute(first_source, "value", "5"));
    assert!(host.set_input_value(first_source, "5"));
    assert!(host.set_attribute(first_target, "class", "target"));
    assert!(host.set_attribute(second_source, "id", "second-range"));
    assert!(host.set_attribute(second_source, "type", "number"));
    assert!(host.set_attribute(second_source, "min", "0"));
    assert!(host.set_attribute(second_source, "max", "10"));
    assert!(host.set_attribute(second_source, "value", "5"));
    assert!(host.set_input_value(second_source, "5"));
    assert!(host.set_attribute(second_target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, first_source));
    assert!(host.append_child(document, first_target));
    assert!(host.append_child(document, second_source));
    assert!(host.append_child(document, second_target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        "#first-range:out-of-range + .target { color: red; }
         #second-range:out-of-range + .target { color: blue; }"
            .into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    for handle in [first_target, second_target, unrelated] {
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

    let first_old_state = engine
        .retained_current_element_state(&host, first_source)
        .expect("retained state should be available after computed style read");
    assert!(host.set_input_value(first_source, "20"));
    let second_old_state = engine
        .retained_current_element_state(&host, second_source)
        .expect("retained state should be available after computed style read");
    assert!(host.set_input_value(second_source, "20"));

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    for (source, old_state) in [
        (first_source, first_old_state),
        (second_source, second_old_state),
    ] {
        engine.invalidate_for_element_state_change_with_old_state(
            &host,
            source,
            StyloElementState::INRANGE
                | StyloElementState::OUTOFRANGE
                | StyloElementState::VALIDITY_STATES,
            Some(old_state),
            &media,
        );
    }
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, first_target)
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, second_target)
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn retained_stylo_invalidator_uses_snapshots_for_mixed_attribute_state_batches() {
    let mut host = test_host();
    let document = host.document_handle();
    let attr_source = host.create_element("div");
    let attr_target = host.create_element("span");
    let state_source = host.create_element("input");
    let state_target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(attr_target, "class", "target"));
    assert!(host.set_attribute(state_source, "id", "range"));
    assert!(host.set_attribute(state_source, "type", "number"));
    assert!(host.set_attribute(state_source, "min", "0"));
    assert!(host.set_attribute(state_source, "max", "10"));
    assert!(host.set_attribute(state_source, "value", "5"));
    assert!(host.set_input_value(state_source, "5"));
    assert!(host.set_attribute(state_target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, attr_source));
    assert!(host.append_child(document, attr_target));
    assert!(host.append_child(document, state_source));
    assert!(host.append_child(document, state_target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        ".marker + .target { color: red; }
         #range:out-of-range + .target { color: blue; }"
            .into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    for handle in [attr_target, state_target, unrelated] {
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

    assert!(host.set_attribute(attr_source, "class", "marker"));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: attr_source,
            name: "class".into(),
            old_value: None,
            new_value: Some("marker".into()),
        }],
        &crate::protocol_types::EmulatedMediaOverrides::default(),
    );

    let old_state = engine
        .retained_current_element_state(&host, state_source)
        .expect("retained state should be available after computed style read");
    assert!(host.set_input_value(state_source, "20"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        state_source,
        StyloElementState::INRANGE
            | StyloElementState::OUTOFRANGE
            | StyloElementState::VALIDITY_STATES,
        Some(old_state),
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, attr_target)
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, state_target)
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn retained_stylo_invalidator_narrows_target_sibling_cache_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("span");
    let source = host.create_element("a");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(source, "id", "current"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source_text = StyloStylesheetSource::new(
        "#current:target + .target { color: green; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source_text.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source_text);
    for handle in [previous, target, unrelated] {
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
    engine.invalidate_for_target_change(&host, None, Some(source), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn retained_stylo_invalidator_uses_snapshots_for_mixed_attribute_focus_batches() {
    let mut host = test_host();
    let document = host.document_handle();
    let attr_scoped_outer = host.create_element("div");
    let attr_scoped = host.create_element("section");
    let attr_scoped_marker = host.create_element("div");
    let attr_scoped_middle = host.create_element("div");
    let attr_scoped_target = host.create_element("p");
    let attr_loose_outer = host.create_element("div");
    let attr_loose = host.create_element("section");
    let attr_loose_marker = host.create_element("div");
    let attr_loose_middle = host.create_element("div");
    let attr_loose_target = host.create_element("p");
    let focus_scoped_outer = host.create_element("div");
    let focus_scoped = host.create_element("section");
    let focus_scoped_marker = host.create_element("button");
    let focus_scoped_middle = host.create_element("div");
    let focus_scoped_target = host.create_element("p");
    let focus_loose_outer = host.create_element("div");
    let focus_loose = host.create_element("section");
    let focus_loose_marker = host.create_element("button");
    let focus_loose_middle = host.create_element("div");
    let focus_loose_target = host.create_element("p");

    for marker in [attr_scoped_marker, attr_loose_marker] {
        assert!(host.set_attribute(marker, "class", "marker"));
    }
    for middle in [attr_scoped_middle, focus_scoped_middle] {
        assert!(host.set_attribute(middle, "class", "scope"));
    }
    for target in [attr_scoped_target, attr_loose_target] {
        assert!(host.set_attribute(target, "class", "attr-target"));
    }
    for target in [focus_scoped_target, focus_loose_target] {
        assert!(host.set_attribute(target, "class", "focus-target"));
    }

    assert!(host.append_child(document, attr_scoped_outer));
    assert!(host.append_child(attr_scoped_outer, attr_scoped));
    assert!(host.append_child(attr_scoped, attr_scoped_marker));
    assert!(host.append_child(attr_scoped_marker, attr_scoped_middle));
    assert!(host.append_child(attr_scoped_middle, attr_scoped_target));
    assert!(host.append_child(document, attr_loose_outer));
    assert!(host.append_child(attr_loose_outer, attr_loose));
    assert!(host.append_child(attr_loose, attr_loose_marker));
    assert!(host.append_child(attr_loose_marker, attr_loose_middle));
    assert!(host.append_child(attr_loose_middle, attr_loose_target));
    assert!(host.append_child(document, focus_scoped_outer));
    assert!(host.append_child(focus_scoped_outer, focus_scoped));
    assert!(host.append_child(focus_scoped, focus_scoped_marker));
    assert!(host.append_child(focus_scoped_marker, focus_scoped_middle));
    assert!(host.append_child(focus_scoped_middle, focus_scoped_target));
    assert!(host.append_child(document, focus_loose_outer));
    assert!(host.append_child(focus_loose_outer, focus_loose));
    assert!(host.append_child(focus_loose, focus_loose_marker));
    assert!(host.append_child(focus_loose_marker, focus_loose_middle));
    assert!(host.append_child(focus_loose_middle, focus_loose_target));
    host.set_active_element_handle(Some(focus_scoped_marker));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "div .scope:where(.marker *) .attr-target { color: red; }
         div .scope:where(:focus *) .focus-target { color: blue; }"
            .into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [
        attr_scoped_target,
        attr_loose_target,
        focus_scoped_target,
        focus_loose_target,
    ] {
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

    assert!(host.set_attribute(attr_scoped_marker, "class", "other"));
    assert!(host.set_attribute(attr_loose_marker, "class", "other"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[
            StyleMutationEffect::Attribute {
                element: attr_scoped_marker,
                name: "class".to_owned(),
                old_value: Some("marker".to_owned()),
                new_value: Some("other".to_owned()),
            },
            StyleMutationEffect::Attribute {
                element: attr_loose_marker,
                name: "class".to_owned(),
                old_value: Some("marker".to_owned()),
                new_value: Some("other".to_owned()),
            },
        ],
        &media,
    );
    host.set_active_element_handle(None);
    engine.invalidate_for_focus_change(&host, Some(focus_scoped_marker), None, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            attr_scoped_target
        )
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            attr_loose_target
        )
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            focus_scoped_target
        )
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            focus_loose_target
        )
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
}
#[test]
fn retained_stylo_invalidator_uses_snapshots_for_mixed_attribute_target_batches() {
    let mut host = test_host();
    let document = host.document_handle();
    let attr_scoped_outer = host.create_element("div");
    let attr_scoped = host.create_element("section");
    let attr_scoped_marker = host.create_element("div");
    let attr_scoped_middle = host.create_element("div");
    let attr_scoped_target = host.create_element("p");
    let attr_loose_outer = host.create_element("div");
    let attr_loose = host.create_element("section");
    let attr_loose_marker = host.create_element("div");
    let attr_loose_middle = host.create_element("div");
    let attr_loose_target = host.create_element("p");
    let target_scoped_outer = host.create_element("div");
    let target_scoped = host.create_element("section");
    let target_scoped_marker = host.create_element("a");
    let target_scoped_middle = host.create_element("div");
    let target_scoped_target = host.create_element("p");
    let target_loose_outer = host.create_element("div");
    let target_loose = host.create_element("section");
    let target_loose_marker = host.create_element("a");
    let target_loose_middle = host.create_element("div");
    let target_loose_target = host.create_element("p");

    for marker in [attr_scoped_marker, attr_loose_marker] {
        assert!(host.set_attribute(marker, "class", "marker"));
    }
    assert!(host.set_attribute(target_scoped_marker, "id", "old"));
    assert!(host.set_attribute(target_loose_marker, "id", "new"));
    for middle in [attr_scoped_middle, target_scoped_middle] {
        assert!(host.set_attribute(middle, "class", "scope"));
    }
    for target in [attr_scoped_target, attr_loose_target] {
        assert!(host.set_attribute(target, "class", "attr-target"));
    }
    for target in [target_scoped_target, target_loose_target] {
        assert!(host.set_attribute(target, "class", "target-target"));
    }

    assert!(host.append_child(document, attr_scoped_outer));
    assert!(host.append_child(attr_scoped_outer, attr_scoped));
    assert!(host.append_child(attr_scoped, attr_scoped_marker));
    assert!(host.append_child(attr_scoped_marker, attr_scoped_middle));
    assert!(host.append_child(attr_scoped_middle, attr_scoped_target));
    assert!(host.append_child(document, attr_loose_outer));
    assert!(host.append_child(attr_loose_outer, attr_loose));
    assert!(host.append_child(attr_loose, attr_loose_marker));
    assert!(host.append_child(attr_loose_marker, attr_loose_middle));
    assert!(host.append_child(attr_loose_middle, attr_loose_target));
    assert!(host.append_child(document, target_scoped_outer));
    assert!(host.append_child(target_scoped_outer, target_scoped));
    assert!(host.append_child(target_scoped, target_scoped_marker));
    assert!(host.append_child(target_scoped_marker, target_scoped_middle));
    assert!(host.append_child(target_scoped_middle, target_scoped_target));
    assert!(host.append_child(document, target_loose_outer));
    assert!(host.append_child(target_loose_outer, target_loose));
    assert!(host.append_child(target_loose, target_loose_marker));
    assert!(host.append_child(target_loose_marker, target_loose_middle));
    assert!(host.append_child(target_loose_middle, target_loose_target));
    assert!(host.set_document_target_element(document, Some(target_scoped_marker)));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/#old").unwrap();
    let source = StyloStylesheetSource::new(
        "div .scope:where(.marker *) .attr-target { color: red; }
         div .scope:where(:target *) .target-target { color: blue; }"
            .into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [
        attr_scoped_target,
        attr_loose_target,
        target_scoped_target,
        target_loose_target,
    ] {
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

    assert!(host.set_attribute(attr_scoped_marker, "class", "other"));
    assert!(host.set_attribute(attr_loose_marker, "class", "other"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[
            StyleMutationEffect::Attribute {
                element: attr_scoped_marker,
                name: "class".to_owned(),
                old_value: Some("marker".to_owned()),
                new_value: Some("other".to_owned()),
            },
            StyleMutationEffect::Attribute {
                element: attr_loose_marker,
                name: "class".to_owned(),
                old_value: Some("marker".to_owned()),
                new_value: Some("other".to_owned()),
            },
        ],
        &media,
    );
    assert!(host.set_document_target_element(document, None));
    engine.invalidate_for_target_change(&host, Some(target_scoped_marker), None, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            attr_scoped_target
        )
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            attr_loose_target
        )
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            target_scoped_target
        )
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            target_loose_target
        )
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
}
#[test]
fn nth_of_child_list_insertion_invalidates_only_changed_and_following_sibling_subtrees() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let first = host.create_element("div");
    let inserted = host.create_element("div");
    let target = host.create_element("div");
    let later = host.create_element("div");
    let unrelated = host.create_element("aside");

    for handle in [first, inserted, target, later] {
        assert!(host.set_attribute(handle, "class", "active"));
    }
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, first));
    assert!(host.append_child(container, target));
    assert!(host.append_child(container, later));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "section > div:nth-child(odd of .active) {
             background-color: rgb(192, 192, 192);
         }
         #unrelated { color: rgb(1, 2, 3); }"
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
            later,
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
            unrelated,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );

    assert!(host.insert_before(container, inserted, Some(target)));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: container,
            added_nodes: vec![inserted],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: Some(first),
            next_sibling: Some(target),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, later));
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
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            later,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgba(0, 0, 0, 0)".into())
    );
}

#[test]
fn nth_of_child_list_removal_invalidates_only_following_sibling_subtrees() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let first = host.create_element("div");
    let removed = host.create_element("div");
    let target = host.create_element("div");
    let later = host.create_element("div");
    let unrelated = host.create_element("aside");

    for handle in [first, removed, target, later] {
        assert!(host.set_attribute(handle, "class", "active"));
    }
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, first));
    assert!(host.append_child(container, removed));
    assert!(host.append_child(container, target));
    assert!(host.append_child(container, later));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "section > div:nth-child(odd of .active) {
             background-color: rgb(192, 192, 192);
         }
         #unrelated { color: rgb(1, 2, 3); }"
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
            target,
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
            later,
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

    let removed_snapshots = removed_element_dependency_snapshots(&host, &[removed]);
    assert!(host.remove_child(container, removed));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: container,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed],
            removed_element_snapshots: removed_snapshots,
            previous_sibling: Some(first),
            next_sibling: Some(target),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, later));
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
        Some("rgba(0, 0, 0, 0)".into())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            later,
            "background-color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(192, 192, 192)".into())
    );
}

#[test]
fn retained_stylo_invalidator_narrows_child_list_inserted_sibling_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let inserted = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(inserted, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
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

    assert!(host.insert_before(document, inserted, Some(target)));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: document,
            added_nodes: vec![inserted],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: Some(previous),
            next_sibling: Some(target),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
}

#[test]
fn retained_stylo_invalidator_keeps_ua_structural_boundary_when_author_source_has_no_target() {
    let mut host = test_host();
    let document = host.document_handle();
    let details = host.create_element("details");
    let first = host.create_element("summary");
    let second = host.create_element("summary");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(unrelated, "class", "unrelated"));
    assert!(host.append_child(document, details));
    assert!(host.append_child(details, first));
    assert!(host.append_child(details, second));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source =
        StyloStylesheetSource::new(".unrelated { color: red; }".into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [first, second, unrelated] {
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
        engine.computed_style_property_value(
            &host,
            &document_url,
            first,
            "display",
            None,
            &inputs,
            None,
        ),
        Some("list-item".into())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            second,
            "display",
            None,
            &inputs,
            None,
        ),
        Some("block".into())
    );

    assert!(host.insert_before(details, second, Some(first)));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[
            StyleMutationEffect::ChildList {
                parent: details,
                added_nodes: Vec::new(),
                removed_nodes: vec![second],
                removed_element_snapshots: removed_element_dependency_snapshots(&host, &[second]),
                previous_sibling: Some(first),
                next_sibling: None,
            },
            StyleMutationEffect::ChildList {
                parent: details,
                added_nodes: vec![second],
                removed_nodes: Vec::new(),
                removed_element_snapshots: Vec::new(),
                previous_sibling: None,
                next_sibling: Some(first),
            },
        ],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, first));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, second));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first,
            "display",
            None,
            &inputs,
            None,
        ),
        Some("block".into())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            second,
            "display",
            None,
            &inputs,
            None,
        ),
        Some("list-item".into())
    );
}

#[test]
fn retained_stylo_invalidator_uses_user_agent_attribute_dependencies() {
    let mut host = test_host();
    let document = host.document_handle();
    let dialog = host.create_element("dialog");
    let unrelated = host.create_element("span");

    assert!(host.append_child(document, dialog));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [dialog, unrelated] {
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
        engine.computed_style_property_value(
            &host,
            &document_url,
            dialog,
            "display",
            None,
            &inputs,
            None,
        ),
        Some("none".into())
    );
    let source_scope = StyleSourceScope::for_document_and_connected_shadow_roots(&host, document);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let dependency_sources = engine.matching_dependency_source_targets_for_document_for_test(
        &host,
        document,
        &source_scope,
        &media,
    );
    assert_eq!(dependency_sources.len(), 1);
    assert!(dependency_sources[0].0.is_user_agent());
    assert!(dependency_sources[0].1.contains(&document));

    assert!(host.set_attribute(dialog, "open", ""));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: dialog,
            name: "open".into(),
            old_value: None,
            new_value: Some(String::new()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, dialog));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated),
        "UA dependency invalidation must preserve unrelated cached styles"
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            dialog,
            "display",
            None,
            &inputs,
            None,
        ),
        Some("block".into())
    );
}

#[test]
fn retained_stylo_invalidator_clears_relative_previous_sibling_for_middle_insert() {
    let mut host = test_host();
    let document = host.document_handle();
    let subject = host.create_element("div");
    let inserted = host.create_element("div");
    let old_next = host.create_element("div");
    let unrelated = host.create_element("div");

    assert!(host.set_attribute(subject, "id", "subject"));
    assert!(host.set_attribute(old_next, "id", "old-next"));
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.append_child(document, subject));
    assert!(host.append_child(document, old_next));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "#subject:has(+ #old-next) { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [subject, unrelated] {
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

    assert!(host.insert_before(document, inserted, Some(old_next)));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: document,
            added_nodes: vec![inserted],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: Some(subject),
            next_sibling: Some(old_next),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_refreshes_has_side_effect_siblings() {
    let mut host = test_host();
    let document = host.document_handle();
    let main = host.create_element("main");
    let previous = host.create_element("div");
    let subject = host.create_element("div");
    let blocker = host.create_element("div");
    let next = host.create_element("div");
    let unrelated = host.create_element("div");

    assert!(host.set_attribute(previous, "id", "prev_sibling"));
    assert!(host.set_attribute(subject, "id", "subject"));
    assert!(host.set_attribute(blocker, "id", "blocks_match"));
    assert!(host.set_attribute(next, "id", "next_sibling"));
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.append_child(document, main));
    assert!(host.append_child(main, previous));
    assert!(host.append_child(main, subject));
    assert!(host.append_child(main, blocker));
    assert!(host.append_child(main, next));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "div, main { color: grey; }
         #subject:has(+ #next_sibling) { color: red; }
         #prev_sibling:has(+ #subject + #next_sibling) { color: green; }"
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
            previous,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(128, 128, 128)".to_owned())
    );
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
        Some("rgb(128, 128, 128)".to_owned())
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

    let removed_snapshots = removed_element_dependency_snapshots(&host, &[blocker]);
    assert!(host.remove_child(main, blocker));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: main,
            added_nodes: Vec::new(),
            removed_nodes: vec![blocker],
            removed_element_snapshots: removed_snapshots,
            previous_sibling: Some(subject),
            next_sibling: Some(next),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            previous,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 128, 0)".to_owned())
    );
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
        Some("rgb(255, 0, 0)".to_owned())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3
    );

    assert!(host.insert_before(main, blocker, Some(next)));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: main,
            added_nodes: vec![blocker],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: Some(subject),
            next_sibling: Some(next),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, subject));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            previous,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(128, 128, 128)".to_owned())
    );
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
        Some("rgb(128, 128, 128)".to_owned())
    );
}

#[test]
fn retained_stylo_invalidator_clears_previous_sibling_for_inserted_last_child_change() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("div");
    let previous = host.create_element("span");
    let first_inserted = host.create_element("span");
    let second_inserted = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, previous));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "span:last-child { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [previous, unrelated] {
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

    assert!(host.append_child(parent, first_inserted));
    assert!(host.append_child(parent, second_inserted));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent,
            added_nodes: vec![first_inserted, second_inserted],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: Some(previous),
            next_sibling: None,
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_merges_inserted_last_child_batches() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("div");
    let previous = host.create_element("span");
    let first_inserted = host.create_element("span");
    let second_inserted = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, previous));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "span:last-child { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [previous, unrelated] {
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

    assert!(host.append_child(parent, first_inserted));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent,
            added_nodes: vec![first_inserted],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: Some(previous),
            next_sibling: None,
        }],
        &media,
    );
    assert!(host.append_child(parent, second_inserted));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent,
            added_nodes: vec![second_inserted],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: Some(first_inserted),
            next_sibling: None,
        }],
        &media,
    );

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        1
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_defers_large_child_list_batches_until_drain() {
    let mut host = test_host();
    let document = host.document_handle();
    let main = host.create_element("main");
    let container = host.create_element("div");
    let subject = host.create_element("div");
    assert!(host.set_attribute(subject, "class", "subject"));
    assert!(host.append_child(document, main));
    assert!(host.append_child(main, container));
    assert!(host.append_child(main, subject));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "main { color: rgb(128, 128, 128); }
         main:has(span) .subject { color: rgb(255, 0, 0); }"
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
            subject,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(128, 128, 128)".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let mut previous_sibling = None;
    for _ in 0..300 {
        let span = host.create_element("span");
        assert!(host.append_child(container, span));
        engine.invalidate_for_mutations(
            &host,
            &[StyleMutationEffect::ChildList {
                parent: container,
                added_nodes: vec![span],
                removed_nodes: Vec::new(),
                removed_element_snapshots: Vec::new(),
                previous_sibling,
                next_sibling: None,
            }],
            &media,
        );
        previous_sibling = Some(span);
    }

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.pending_structural_style_mutation_effect_count_for_document_for_test(document),
        300
    );

    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0
    );
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
        Some("rgb(255, 0, 0)".into())
    );
}

#[test]
fn retained_stylo_invalidator_clears_previous_sibling_for_removed_last_child_change() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("div");
    let previous = host.create_element("span");
    let removed = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, previous));
    assert!(host.append_child(parent, removed));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "span:last-child { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [previous, unrelated] {
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

    let removed_snapshots = removed_element_dependency_snapshots(&host, &[removed]);
    assert!(host.remove_child(parent, removed));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed],
            removed_element_snapshots: removed_snapshots,
            previous_sibling: Some(previous),
            next_sibling: None,
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, previous));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn retained_stylo_invalidator_narrows_child_list_removed_sibling_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let removed = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(removed, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, removed));
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

    assert!(host.remove_child(document, removed));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: document,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed],
            removed_element_snapshots: removed_element_dependency_snapshots(&host, &[removed]),
            previous_sibling: Some(previous),
            next_sibling: Some(target),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn retained_stylo_invalidator_narrows_child_list_replacement_sibling_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let inserted = host.create_element("div");
    let removed = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(inserted, "class", "marker"));
    assert!(host.set_attribute(removed, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, removed));
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

    assert!(host.insert_before(document, inserted, Some(removed)));
    assert!(host.remove_child(document, removed));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: document,
            added_nodes: vec![inserted],
            removed_nodes: vec![removed],
            removed_element_snapshots: removed_element_dependency_snapshots(&host, &[removed]),
            previous_sibling: Some(previous),
            next_sibling: Some(target),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
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
    let inputs = FullStyleWorldSnapshot::default();
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
        name: "class".into(),
        old_value: None,
        new_value: Some("host".into()),
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
            if roots.contains(&part)
                && !roots.contains(&outside)
                && !roots.contains(&unrelated_shadow_child)
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
