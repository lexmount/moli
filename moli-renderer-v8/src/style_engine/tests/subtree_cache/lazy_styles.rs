use super::*;

#[test]
fn computed_style_is_published_on_canonical_element_data_and_reused() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        ".target { color: rgb(1, 2, 3); }".into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();

    let first = computed_style_snapshot_for_test(&engine, &host, &document_url, target, &inputs);
    let retained_first = retained_primary_style_for_test(&engine, &host, target)
        .expect("the target's computed style must live on canonical ElementData");
    assert!(ServoArc::ptr_eq(&first.computed_values(), &retained_first));
    let publication_generation =
        engine.computed_style_publication_generation_for_document_for_test(document);

    // The adapter rejects nested bindings. A cache hit can read the
    // published styles even while another scope holds the binding.
    let second = engine.dom_adapter.with_bound_host(&host, |binding| {
        let style =
            computed_style_snapshot_for_test(&engine, &host, &document_url, target, &inputs);
        assert_eq!(binding.bound_node_count(), 0);
        style
    });
    let retained_second = retained_primary_style_for_test(&engine, &host, target)
        .expect("a clean read must retain canonical ElementData");
    assert!(ServoArc::ptr_eq(
        &first.computed_values(),
        &second.computed_values()
    ));
    assert!(ServoArc::ptr_eq(&retained_first, &retained_second));
    assert_eq!(
        engine.computed_style_publication_generation_for_document_for_test(document),
        publication_generation,
        "reading clean canonical style must not republish a cache entry"
    );
}

#[test]
fn presentational_hint_restyles_self_and_recascades_inherited_values() {
    let mut host = test_host();
    let document = host.document_handle();
    let heading = host.create_element("h1");
    let child = host.create_element("span");
    assert!(host.append_child(document, heading));
    assert!(host.append_child(heading, child));
    assert!(host.set_attribute(heading, "align", "left"));

    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    let alignment = |engine: &MoliStyleEngine, host: &DomHost| {
        engine
            .computed_style_property_value(host, &url, child, "text-align", None, &inputs, None)
            .unwrap()
    };
    assert_eq!(alignment(&engine, &host), "left");
    let mutation = host.set_attribute_mutation_outcome(heading, "align", "right");
    engine.invalidate_dom_mutations(
        &host,
        &StyleMutationEffect::from_dom_mutation_effects(mutation.effects()),
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(element_style_is_dirty_for_test(&engine, &host, heading));
    assert!(!element_style_is_dirty_for_test(&engine, &host, child));
    assert_eq!(
        engine.retained_style_invalidation_root_count_for_document_for_test(document),
        0,
    );
    assert_eq!(alignment(&engine, &host), "right");
}

#[test]
fn lazy_subtree_invalidation_avoids_eager_scans_and_memoizes_paths() {
    const UNRELATED_COUNT: usize = 256;

    let mut host = test_host();
    let document = host.document_handle();
    let affected_root = host.create_element("section");
    let affected_leaf = host.create_element("span");
    let affected_sibling = host.create_element("span");
    let unrelated_root = host.create_element("main");
    assert!(host.append_child(document, affected_root));
    assert!(host.append_child(affected_root, affected_leaf));
    assert!(host.append_child(affected_root, affected_sibling));
    assert!(host.append_child(document, unrelated_root));
    let unrelated = (0..UNRELATED_COUNT)
        .map(|_| {
            let element = host.create_element("i");
            assert!(host.append_child(unrelated_root, element));
            element
        })
        .collect::<Vec<_>>();

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [affected_leaf, affected_sibling]
        .into_iter()
        .chain(unrelated.iter().copied())
    {
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
        UNRELATED_COUNT + 2
    );

    let path_visits_before =
        engine.style_invalidation_path_node_visit_count_for_document_for_test(document);
    engine.invalidate_style_subtree(&host, affected_root);
    assert_eq!(
        engine.style_invalidation_path_node_visit_count_for_document_for_test(document),
        path_visits_before,
        "recording an invalidation root must not inspect any published descendant"
    );
    assert_eq!(
        engine.retained_style_invalidation_root_count_for_document_for_test(document),
        1
    );
    assert!(engine.style_invalidation_generation_for_document_for_test(document) > 0);
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        UNRELATED_COUNT + 2,
        "all descendant publication entries remain untouched at mutation time"
    );

    let resolutions_before = engine.element_style_resolution_count_for_document_for_test(document);
    for &handle in &unrelated {
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
        engine.element_style_resolution_count_for_document_for_test(document),
        resolutions_before,
        "an unrelated branch must remain canonical"
    );
    let unrelated_path_visits = engine
        .style_invalidation_path_node_visit_count_for_document_for_test(document)
        .saturating_sub(path_visits_before);
    assert!(
        unrelated_path_visits <= (UNRELATED_COUNT as u64).saturating_mul(2).saturating_add(3),
        "memoized parent breadcrumbs should make a sibling sweep linear; visited {unrelated_path_visits} nodes"
    );

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                affected_leaf,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    let resolutions_after_first_affected_read =
        engine.element_style_resolution_count_for_document_for_test(document);
    assert!(resolutions_after_first_affected_read > resolutions_before);
    assert!(
        engine
            .computed_style_cache_contains_handle_for_document_for_test(document, affected_sibling),
        "consuming one element must not enumerate or evict an unread sibling"
    );
    let visits_before_repeat =
        engine.style_invalidation_path_node_visit_count_for_document_for_test(document);
    engine.dom_adapter.with_bound_host(&host, |binding| {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    affected_leaf,
                    "display",
                    None,
                    &inputs,
                    None,
                )
                .is_some()
        );
        assert_eq!(binding.bound_node_count(), 0);
    });
    assert_eq!(
        engine.element_style_resolution_count_for_document_for_test(document),
        resolutions_after_first_affected_read,
        "a second clean observation must reuse the refreshed ElementData"
    );
    assert_eq!(
        engine.style_invalidation_path_node_visit_count_for_document_for_test(document),
        visits_before_repeat,
        "a memoized generation requires no validation path"
    );

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                affected_sibling,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    assert!(
        engine.element_style_resolution_count_for_document_for_test(document)
            > resolutions_after_first_affected_read,
        "the retained root history must independently validate an unread sibling"
    );
}

#[test]
fn repeated_lazy_invalidation_advances_past_previously_validated_elements() {
    let mut host = test_host();
    let document = host.document_handle();
    let root = host.create_element("section");
    let leaf = host.create_element("span");
    assert!(host.append_child(document, root));
    assert!(host.append_child(root, leaf));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    let read_leaf = |engine: &mut MoliStyleEngine| {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    leaf,
                    "display",
                    None,
                    &inputs,
                    None,
                )
                .is_some()
        );
    };

    read_leaf(&mut engine);
    engine.invalidate_style_subtree(&host, root);
    let first_generation = engine.style_invalidation_generation_for_document_for_test(document);
    read_leaf(&mut engine);
    let resolutions_after_first_generation =
        engine.element_style_resolution_count_for_document_for_test(document);

    engine.invalidate_style_subtree(&host, root);
    assert!(
        engine.style_invalidation_generation_for_document_for_test(document) > first_generation
    );
    read_leaf(&mut engine);
    assert!(
        engine.element_style_resolution_count_for_document_for_test(document)
            > resolutions_after_first_generation,
        "a new root generation must invalidate path stamps from the previous observation"
    );
}

#[test]
fn lazy_invalidated_deep_subtree_does_not_rewalk_validated_ancestors() {
    const DEPTH: usize = 256;

    let mut host = test_host();
    let document = host.document_handle();
    let root = host.create_element("section");
    assert!(host.append_child(document, root));
    let mut chain = vec![root];
    let mut parent = root;
    for _ in 1..DEPTH {
        let child = host.create_element("div");
        assert!(host.append_child(parent, child));
        chain.push(child);
        parent = child;
    }

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                parent,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );

    engine.invalidate_style_subtree(&host, root);
    let ancestor_visits_before =
        engine.ancestor_style_validation_visit_count_for_document_for_test(document);
    let resolutions_before = engine.element_style_resolution_count_for_document_for_test(document);
    for &handle in &chain {
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

    let ancestor_visits = engine
        .ancestor_style_validation_visit_count_for_document_for_test(document)
        .saturating_sub(ancestor_visits_before);
    assert!(
        ancestor_visits <= DEPTH as u64,
        "parent-before-child observation should reuse the nearest validated breadcrumb; visited {ancestor_visits} ancestors"
    );
    assert_eq!(
        engine
            .element_style_resolution_count_for_document_for_test(document)
            .saturating_sub(resolutions_before),
        DEPTH as u64,
        "every dirty element is recascaded exactly once"
    );
}

#[test]
fn primary_snapshot_carries_applicable_eager_pseudos_without_forcing_absent_ones() {
    let mut host = test_host();
    let document = host.document_handle();
    let generated = host.create_element("div");
    let plain = host.create_element("div");
    assert!(host.set_attribute(generated, "class", "generated"));
    assert!(host.append_child(document, generated));
    assert!(host.append_child(document, plain));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ".generated::before { content: 'before'; }\n\
             .generated::after { content: 'after'; }"
                .into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 1,
        ))),
    );

    let before = engine.element_style_resolution_count_for_document_for_test(document);
    let generated_snapshot =
        computed_style_snapshot_for_test(&engine, &host, &document_url, generated, &inputs);
    let (_, generated_before, generated_after) = generated_snapshot.into_element_computed_values();
    assert!(generated_before.is_some());
    assert!(generated_after.is_some());
    assert_eq!(
        engine.element_style_resolution_count_for_document_for_test(document),
        before + 1,
        "the primary resolve already cascades both eager pseudo styles"
    );

    let before = engine.element_style_resolution_count_for_document_for_test(document);
    let plain_snapshot =
        computed_style_snapshot_for_test(&engine, &host, &document_url, plain, &inputs);
    let (_, plain_before, plain_after) = plain_snapshot.into_element_computed_values();
    assert!(plain_before.is_none());
    assert!(plain_after.is_none());
    assert_eq!(
        engine.element_style_resolution_count_for_document_for_test(document),
        before + 1,
        "observing absent eager pseudos must not force two extra resolutions"
    );

    let world = engine.world_for_document(document);
    let writes = world.pseudo_style_cache.write_generation();
    crate::style_engine::take_style_resolution_context_count_for_test();
    for (pseudo, content) in [("before", "\"before\""), ("after", "\"after\"")] {
        assert_eq!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    generated,
                    "content",
                    Some(pseudo),
                    &inputs,
                    None,
                )
                .as_deref(),
            Some(content)
        );
    }
    assert_eq!(world.pseudo_style_cache.write_generation(), writes);
    assert_eq!(
        crate::style_engine::take_style_resolution_context_count_for_test(),
        0
    );
}

#[test]
fn scoped_invalidation_marks_canonical_style_dirty_and_preserves_clean_elements() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let parent = host.create_element("section");
    let target = host.create_element("li");
    assert!(host.set_attribute(target, "style", "color: rgb(1, 2, 3)"));
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, target));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "li { display: list-item; } li::marker { color: rgb(7, 8, 9); }".into(),
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    let parent_before =
        computed_style_snapshot_for_test(&engine, &host, &document_url, parent, &inputs)
            .computed_values();
    let target_before =
        computed_style_snapshot_for_test(&engine, &host, &document_url, target, &inputs)
            .computed_values();
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
        Some("rgb(1, 2, 3)".into())
    );
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
                "color",
                Some("marker"),
                &inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_handle_for_document_for_test(document, target),
        2,
        "the publication index should contain primary and pseudo entries"
    );

    assert!(host.set_attribute(target, "style", "color: rgb(4, 5, 6)"));
    engine.invalidate_inline_style_subtree(&host, target);

    let retained_dirty_target = retained_primary_style_for_test(&engine, &host, target)
        .expect("dirty ElementData should retain its last published values");
    assert!(ServoArc::ptr_eq(&target_before, &retained_dirty_target));
    assert!(element_style_is_dirty_for_test(&engine, &host, target));
    let retained_parent = retained_primary_style_for_test(&engine, &host, parent)
        .expect("the unaffected parent must retain its computed style");
    assert!(ServoArc::ptr_eq(&parent_before, &retained_parent));
    assert!(!element_style_is_dirty_for_test(&engine, &host, parent));
    assert_eq!(
        engine.computed_style_cache_entry_count_for_handle_for_document_for_test(document, target),
        0,
        "dirtying an element must evict both its publication marker and pseudo sidecar"
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
        Some("rgb(4, 5, 6)".into())
    );
    let target_after = retained_primary_style_for_test(&engine, &host, target)
        .expect("the recomputed style must be republished on ElementData");
    assert!(!ServoArc::ptr_eq(&target_before, &target_after));
    assert!(!element_style_is_dirty_for_test(&engine, &host, target));
}

#[test]
fn inherited_ancestor_change_lazily_refreshes_descendant_style() {
    let mut host = test_host();
    let document = host.document_handle();
    let ancestor = host.create_element("section");
    let descendant = host.create_element("span");
    let unrelated = host.create_element("aside");
    let source_text = ".theme { color: rgb(1, 2, 3); }";
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, descendant));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(source_text.into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
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
        namespace: None,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(
        element_style_is_dirty_for_test(&engine, &host, ancestor),
        "the directly matched ancestor keeps Stylo's precise restyle hint"
    );
    assert!(
        !element_style_is_dirty_for_test(&engine, &host, descendant),
        "inherited work is computed when the dirty ancestor is read"
    );
    assert_eq!(
        engine.retained_style_invalidation_root_count_for_document_for_test(document),
        0,
        "exact selector invalidation is carried by the native hint"
    );

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "subtree invalidation must not enumerate and evict published descendants"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, descendant),
        "the descendant remains published until it is observed"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    let resolutions_before = engine.element_style_resolution_count_for_document_for_test(document);
    let descendant_color = engine.computed_style_property_value(
        &host,
        &document_url,
        descendant,
        "color",
        None,
        &inputs,
        None,
    );
    assert!(
        !element_style_is_dirty_for_test(&engine, &host, ancestor),
        "observing the descendant must recascade its dirty ancestor first"
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            ancestor,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into()),
        "the ancestor recascade must publish the newly matched rule"
    );
    assert_eq!(descendant_color, Some("rgb(1, 2, 3)".into()));
    assert!(
        engine.element_style_resolution_count_for_document_for_test(document) > resolutions_before,
        "the first descendant observation must consume the retained dirty root"
    );
}

#[test]
fn custom_property_ancestor_change_lazily_refreshes_descendant_style() {
    let mut host = test_host();
    let document = host.document_handle();
    let ancestor = host.create_element("section");
    let descendant = host.create_element("span");
    let unrelated = host.create_element("aside");
    let source_text = ".theme { --accent: rgb(1, 2, 3); } span { color: var(--accent, black); }";
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, descendant));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(source_text.into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
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
        namespace: None,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "subtree invalidation must not enumerate var() consumers"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, descendant),
        "the descendant remains published until it is observed"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            descendant,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
}

#[test]
fn reset_style_change_recascades_only_children_that_explicitly_inherit() {
    let mut host = test_host();
    let document = host.document_handle();
    let ancestor = host.create_element("section");
    let descendant = host.create_element("span");
    let inheriting = host.create_element("span");
    let source_text = ".theme { background-color: rgb(1, 2, 3); transition-duration: 2s; }\n\
                       .inherits { background-color: inherit; transition-duration: inherit; }";
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, descendant));
    assert!(host.set_attribute(inheriting, "class", "inherits"));
    assert!(host.append_child(ancestor, inheriting));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(source_text.into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [descendant, inheriting] {
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
    let descendant_before = retained_primary_style_for_test(&engine, &host, descendant)
        .expect("descendant style should be retained");

    assert!(host.set_attribute(ancestor, "class", "theme"));
    let effects = [StyleMutationEffect::Attribute {
        element: ancestor,
        namespace: None,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "invalidation must leave the existing styles available for comparison"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, descendant),
        "a reset-style change should leave the independent child style intact"
    );
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                descendant,
                "background-color",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    let descendant_after = retained_primary_style_for_test(&engine, &host, descendant)
        .expect("observed descendant style should be republished");
    assert!(
        ServoArc::ptr_eq(&descendant_before, &descendant_after),
        "the independent child should reuse its canonical style"
    );
    for (property, expected) in [
        ("background-color", "rgb(1, 2, 3)"),
        ("transition-duration", "2s"),
    ] {
        assert_eq!(
            engine.computed_style_property_value(
                &host,
                &document_url,
                inheriting,
                property,
                None,
                &inputs,
                None,
            ),
            Some(expected.into()),
        );
    }
}

#[test]
fn lazy_child_restyle_keeps_earlier_inheritance_and_stops_at_an_override() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let inherited = host.create_element("span");
    let override_color = host.create_element("aside");
    let grandchild = host.create_element("span");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, inherited));
    assert!(host.append_child(parent, override_color));
    assert!(host.append_child(override_color, grandchild));
    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "section { color: red; } section.blue { color: blue; }\n\
         section.bg { background-color: lime; } aside { color: green; }"
            .into(),
        url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![source],
        ..Default::default()
    };
    let color = |engine: &MoliStyleEngine, host: &DomHost, target| {
        engine.computed_style_property_value(host, &url, target, "color", None, &inputs, None)
    };
    assert_eq!(
        color(&engine, &host, inherited).as_deref(),
        Some("rgb(255, 0, 0)")
    );
    assert_eq!(
        color(&engine, &host, grandchild).as_deref(),
        Some("rgb(0, 128, 0)")
    );
    let grandchild_before = retained_primary_style_for_test(&engine, &host, grandchild).unwrap();

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    for (old, new) in [(None, "blue"), (Some("blue"), "blue bg")] {
        assert!(host.set_attribute(parent, "class", new));
        engine.invalidate_for_mutations(
            &host,
            &[StyleMutationEffect::Attribute {
                element: parent,
                namespace: None,
                name: "class".into(),
                old_value: old.map(Into::into),
                new_value: Some(new.into()),
            }],
            &media,
        );
        engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
        assert_eq!(
            color(&engine, &host, parent).as_deref(),
            Some("rgb(0, 0, 255)")
        );
    }

    assert_eq!(
        color(&engine, &host, inherited).as_deref(),
        Some("rgb(0, 0, 255)")
    );
    assert_eq!(
        color(&engine, &host, grandchild).as_deref(),
        Some("rgb(0, 128, 0)")
    );
    let grandchild_after = retained_primary_style_for_test(&engine, &host, grandchild).unwrap();
    assert!(ServoArc::ptr_eq(&grandchild_before, &grandchild_after));
}

#[test]
fn branch_requirement_budget_preserves_deferred_inheritance() {
    use crate::style_engine::style_paths::MAX_CACHED_BRANCH_REQUIREMENTS;

    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let unrelated = host.create_element("aside");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(document, unrelated));
    let branches = (0..MAX_CACHED_BRANCH_REQUIREMENTS + 1)
        .map(|_| {
            let handle = host.create_element("div");
            assert!(host.append_child(parent, handle));
            handle
        })
        .collect::<Vec<_>>();
    let deferred = host.create_element("span");
    assert!(host.append_child(branches[0], deferred));

    let engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![StyloStylesheetSource::new(
            "section { color: red; } section.blue { color: blue; } aside { color: green; }".into(),
            url.clone(),
        )],
        ..Default::default()
    };
    let color = |host: &DomHost, handle| {
        engine.computed_style_property_value(host, &url, handle, "color", None, &inputs, None)
    };
    let red = Some("rgb(255, 0, 0)");
    let blue = Some("rgb(0, 0, 255)");
    let green = Some("rgb(0, 128, 0)");
    assert_eq!(color(&host, deferred).as_deref(), red);
    assert_eq!(color(&host, unrelated).as_deref(), green);
    for &branch in &branches {
        assert_eq!(color(&host, branch).as_deref(), red);
    }

    let world = engine.world_for_document(document);
    let invalidation = &world.document_state.style_paths;
    let before = invalidation.generation();
    assert!(host.set_attribute(parent, "class", "blue"));
    assert!(engine.dom_adapter.mark_element_style_dirty(
        parent,
        style::invalidation::element::restyle_hints::RestyleHint::RESTYLE_SELF,
    ));
    world.document_state.record_style_targets(
        &host,
        &engine.dom_adapter,
        document,
        [(parent, StyleInvalidationScope::Element)],
    );
    let unrelated_is_current = || {
        engine
            .dom_adapter
            .style_path(document, unrelated)
            .is_some_and(|path| path.observed_at == invalidation.generation())
    };
    assert_eq!(color(&host, unrelated).as_deref(), green);
    assert!(unrelated_is_current());
    assert!(invalidation.can_compare_after(before));
    assert_eq!(invalidation.branch_requirement_count(), 0);

    let mut peak = 0;
    for branch in branches {
        assert_eq!(color(&host, branch).as_deref(), blue);
        peak = peak.max(invalidation.branch_requirement_count());
        assert!(peak <= MAX_CACHED_BRANCH_REQUIREMENTS);
    }
    assert_eq!(peak, MAX_CACHED_BRANCH_REQUIREMENTS);
    assert!(!invalidation.can_compare_after(before));
    assert!(unrelated_is_current());
    assert_eq!(color(&host, deferred).as_deref(), blue);
    assert_eq!(color(&host, unrelated).as_deref(), green);
}

#[test]
fn shadow_host_inherited_change_lazily_refreshes_shadow_descendant() {
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let shadow_child = host.create_element("span");
    let unrelated = host.create_element("aside");
    let source_text = ".theme { color: rgb(1, 2, 3); }";
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(shadow_root, shadow_child));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(source_text.into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
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
        namespace: None,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "shadow descendants must not be enumerated during invalidation"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child),
        "the shadow descendant remains published until observed"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            shadow_child,
            "color",
            None,
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
}

#[test]
fn lazy_pseudo_inherited_change_is_evicted_when_owner_is_observed() {
    let mut host = test_host();
    let document = host.document_handle();
    let ancestor = host.create_element("section");
    let list_item = host.create_element("li");
    let unrelated = host.create_element("aside");
    let source_text =
        ".theme { color: rgb(1, 2, 3); } li { display: list-item; } li::marker { color: inherit; }";
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, list_item));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(source_text.into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
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
        namespace: None,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3,
        "mutation-time invalidation must retain the descendant pseudo sidecar"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, list_item),
        "the owner and pseudo are evicted together only when the owner is demanded"
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
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            list_item,
            "color",
            Some("marker"),
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        engine
            .computed_style_cache_entry_count_for_handle_for_document_for_test(document, list_item),
        2,
        "the refreshed primary and pseudo must be republished"
    );
    let read_marker = || {
        engine
            .computed_pseudo_style_snapshot_from_current_observation(
                &host,
                &document_url,
                list_item,
                "marker",
                StyleSourceDocumentContext::for_root_document(document),
                document,
            )
            .expect("the refreshed marker should be cached")
            .computed_values()
    };
    let visits_before_repeat =
        engine.style_invalidation_path_node_visit_count_for_document_for_test(document);
    engine.dom_adapter.with_bound_host(&host, |binding| {
        assert!(ServoArc::ptr_eq(&read_marker(), &read_marker()));
        assert_eq!(binding.bound_node_count(), 0);
    });
    assert_eq!(
        engine.style_invalidation_path_node_visit_count_for_document_for_test(document),
        visits_before_repeat
    );
}

#[test]
fn lazy_pseudo_custom_property_change_is_evicted_when_owner_is_observed() {
    let mut host = test_host();
    let document = host.document_handle();
    let ancestor = host.create_element("section");
    let list_item = host.create_element("li");
    let unrelated = host.create_element("aside");
    let source_text = ".theme { --marker-color: rgb(1, 2, 3); } li { display: list-item; } li::marker { color: var(--marker-color, black); }";
    assert!(host.append_child(document, ancestor));
    assert!(host.append_child(ancestor, list_item));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(source_text.into(), document_url.clone());
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
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
        namespace: None,
        name: "class".into(),
        old_value: None,
        new_value: Some("theme".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        3,
        "mutation-time invalidation must retain the descendant pseudo sidecar"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, list_item),
        "the pseudo var() consumer remains published until its owner is demanded"
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
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            list_item,
            "color",
            Some("marker"),
            &inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
}
