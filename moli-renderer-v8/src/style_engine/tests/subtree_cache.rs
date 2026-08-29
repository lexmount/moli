use super::*;

fn assert_only_source_document_is_dirty(
    engine: &MoliStyleEngine,
    dirty_document: DomHandle,
    clean_document: DomHandle,
) {
    assert!(
        !engine
            .source_dirty_scope_reasons_for_document_for_test(dirty_document)
            .is_empty(),
        "the source owner document must carry pending style work"
    );
    assert!(
        engine
            .source_dirty_scope_reasons_for_document_for_test(clean_document)
            .is_empty(),
        "an unrelated document must remain clean"
    );
}

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

    let second = computed_style_snapshot_for_test(&engine, &host, &document_url, target, &inputs);
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
    assert_eq!(
        engine.element_style_resolution_count_for_document_for_test(document),
        resolutions_after_first_affected_read,
        "a second clean observation must reuse the refreshed ElementData"
    );
    assert_eq!(
        engine.style_invalidation_path_node_visit_count_for_document_for_test(document),
        visits_before_repeat + 1,
        "a fully memoized target requires one O(1) generation check"
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
        "the descendant is represented by the lazy dirty-root generation"
    );
    assert!(
        engine.retained_style_invalidation_root_count_for_document_for_test(document) > 0,
        "the finalized invalidation must publish a lazy subtree root"
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
fn non_inherited_exact_change_lazily_recascades_descendant_conservatively() {
    let mut host = test_host();
    let document = host.document_handle();
    let ancestor = host.create_element("section");
    let descendant = host.create_element("span");
    let unrelated = host.create_element("aside");
    let source_text = ".theme { background-color: rgb(1, 2, 3); }";
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
        "subtree invalidation must not enumerate published descendants"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, descendant),
        "the conservative descendant recascade is deferred until observation"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
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
        !ServoArc::ptr_eq(&descendant_before, &descendant_after),
        "the lazy subtree contract remains inheritance-safe even for a non-inherited change"
    );
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
    engine.set_document_adopted_style_sheet_sources(document, vec![first_source.clone()]);
    let first_source_ids = engine.document_adopted_style_sheet_source_ids_for_test(document);
    let mut first_inputs = FullStyleWorldSnapshot::default();
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
    engine.set_document_adopted_style_sheet_sources(document, vec![second_source.clone()]);
    let second_source_ids = engine.document_adopted_style_sheet_source_ids_for_test(document);

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        generation_after_first_build
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2,
        "a source mutation must delay invalidation until the next observation"
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
    let mut second_inputs = FullStyleWorldSnapshot::default();
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
fn document_source_update_preserves_shadow_scope_cascade_data() {
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
    engine.set_document_adopted_style_sheet_sources(document, vec![first_document_source.clone()]);
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

    let mut active_inputs = FullStyleWorldSnapshot::default();
    active_inputs
        .document_stylesheet_sources
        .push(first_document_source);
    active_inputs
        .shadow_stylesheet_sources
        .push((active_shadow_root, vec![active_shadow_source]));
    let active_key = StyleWorldKey::new(&active_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, active_key, &active_inputs);

    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .shadow_stylesheet_sources
        .push((detached_shadow_root, vec![detached_shadow_source]));
    let detached_key = StyleWorldKey::new(&detached_inputs, None);
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
            active_cascade_data.clone(),
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

    engine.set_document_adopted_style_sheet_sources(document, vec![second_document_source.clone()]);

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

    active_inputs.document_stylesheet_sources.clear();
    active_inputs
        .document_stylesheet_sources
        .push(second_document_source);
    let active_key = StyleWorldKey::new(&active_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, active_key, &active_inputs);
    let active_cascade_data_after =
        engine.with_retained_style_system_for_document_for_test(document, |retained| {
            retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == active_shadow_root)
                .expect("active retained system should keep the active shadow root")
                .1
                .clone()
        });
    assert!(ServoArc::ptr_eq(
        &active_cascade_data,
        &active_cascade_data_after
    ));
}

#[test]
fn clear_all_fallback_clears_element_styles_without_replacing_the_style_world() {
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
    engine.set_document_adopted_style_sheet_sources(document, vec![first_source.clone()]);
    let mut first_inputs = FullStyleWorldSnapshot::default();
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
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);
    let rebuilds = engine.retained_style_system_rebuild_count_for_document_for_test(document);
    let updates = engine.retained_style_system_update_count_for_document_for_test(document);

    let second_source = StyloStylesheetSource::new(
        "section { color: rgb(4, 5, 6); }".to_owned(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![second_source.clone()]);
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

    let mut second_inputs = FullStyleWorldSnapshot::default();
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
        generation_after_first_build,
        "a clear-all element fallback must not replace the document style world"
    );
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        updates + 1
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
    let mut first_inputs = FullStyleWorldSnapshot::default();
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
        2,
        "an owner stylesheet mutation must wait for the next observation"
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
    let mut second_inputs = FullStyleWorldSnapshot::default();
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

    let mut inputs = FullStyleWorldSnapshot::default();
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
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
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
    let mut first_inputs = FullStyleWorldSnapshot::default();
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
        2,
        "batched document and shadow mutations must wait for one observation"
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
    let mut second_inputs = FullStyleWorldSnapshot::default();
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
fn removed_linked_stylesheet_owner_lifecycle_marks_owner_document_dirty() {
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

    let inputs = FullStyleWorldSnapshot::default();
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
        1,
        "removing a stylesheet owner delays style invalidation until observation"
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

    let inputs = FullStyleWorldSnapshot::default();
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
        1,
        "the final owner lifecycle state is applied at the observation boundary"
    );
}

#[test]
fn inactive_linked_stylesheet_owner_lifecycle_marks_owner_document_dirty() {
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

    let inputs = FullStyleWorldSnapshot::default();
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
        1,
        "disabling a stylesheet owner delays style invalidation until observation"
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
    let inputs = FullStyleWorldSnapshot::default();
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
        1,
        "installing an explicit source only marks the owner world dirty"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
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
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .document_stylesheet_sources
        .push(first_source);
    let active_inputs = FullStyleWorldSnapshot::default();

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
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .document_stylesheet_sources
        .push(linked_source.clone());
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
        1,
        "the detached owner world invalidates on its next observation"
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
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .document_stylesheet_sources
        .push(first_source);
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
        1,
        "the detached linked-source world invalidates at observation time"
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
    let mut first_inputs = FullStyleWorldSnapshot::default();
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
        2,
        "a linked stylesheet revision must wait for the next observation"
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
    let mut second_inputs = FullStyleWorldSnapshot::default();
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
        engine.computed_style_cache_contains_handle_for_document_for_test(document, shadow_child)
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

    let mut active_inputs = FullStyleWorldSnapshot::default();
    active_inputs.document_stylesheet_sources.push(first_source);
    let detached_inputs = FullStyleWorldSnapshot::default();

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
        1,
        "the current owner document invalidates at observation time"
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
    assert_only_source_document_is_dirty(&engine, document, detached_document);
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
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .document_stylesheet_sources
        .push(first_source);
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
        1,
        "the explicit owner document invalidates at observation time"
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

    let active_inputs = FullStyleWorldSnapshot::default();
    let mut detached_inputs = FullStyleWorldSnapshot::default();
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
        1,
        "capturing the explicit owner only marks that world dirty"
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
        1,
        "the installed source is applied at the next observation"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
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
        1,
        "the installed disabled source is applied at the next observation"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
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
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .document_stylesheet_sources
        .push(first_source);
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
            &FullStyleWorldSnapshot::default(),
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
            &FullStyleWorldSnapshot::default(),
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
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .shadow_stylesheet_sources
        .push((detached_shadow_root, detached_shadow_sources));
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
        1,
        "the detached shadow world invalidates at observation time"
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

    engine.set_document_adopted_style_sheet_sources(
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
    let document_adopted_ids = engine.document_adopted_style_sheet_source_ids_for_test(document);
    let shadow_adopted_ids =
        engine.shadow_root_adopted_style_sheet_source_ids_for_test(&host, shadow_root);

    for id in [
        document_style_id,
        linked_style_id,
        shadow_style_id.clone(),
        document_adopted_ids[0].clone(),
        document_adopted_ids[1].clone(),
        shadow_adopted_ids[0].clone(),
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
fn style_world_identity_changes_when_screen_size_changes() {
    let inputs = FullStyleWorldSnapshot::default();
    let viewport =
        StyleViewport::new(Some(800.0), Some(600.0)).with_screen_size(Some(1920.0), Some(1080.0));
    let next_viewport =
        StyleViewport::new(Some(800.0), Some(600.0)).with_screen_size(Some(1366.0), Some(768.0));

    assert_ne!(
        StyleWorldKey::new(&inputs, viewport),
        StyleWorldKey::new(&inputs, next_viewport)
    );
}

#[test]
fn style_world_identity_does_not_hash_stylesheet_text() {
    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { color: green; }".to_owned(),
            document_url.clone(),
        ));
    let mut next_inputs = FullStyleWorldSnapshot::default();
    next_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { color: blue; }".to_owned(),
            document_url.clone(),
        ));

    assert_eq!(
        StyleWorldKey::new(&inputs, None),
        StyleWorldKey::new(&next_inputs, None)
    );
}
#[test]
fn style_world_identity_does_not_hash_stylesheet_base_urls() {
    let old_base = url::Url::parse("https://example.test/assets/app.css").unwrap();
    let next_base = url::Url::parse("https://cdn.example.test/assets/app.css").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { background-image: url(icon.png); }".to_owned(),
            old_base,
        ));
    let mut next_inputs = FullStyleWorldSnapshot::default();
    next_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".probe { background-image: url(icon.png); }".to_owned(),
            next_base,
        ));

    assert_eq!(
        StyleWorldKey::new(&inputs, None),
        StyleWorldKey::new(&next_inputs, None)
    );
}
#[test]
fn style_world_identity_changes_when_document_quirks_mode_changes() {
    let standards_inputs = FullStyleWorldSnapshot::default();
    let quirks_inputs = FullStyleWorldSnapshot {
        quirks_mode: style::context::QuirksMode::Quirks,
        ..Default::default()
    };

    assert_ne!(
        StyleWorldKey::new(&standards_inputs, None),
        StyleWorldKey::new(&quirks_inputs, None)
    );
}

#[test]
fn style_world_identity_mismatch_trace_records_changed_dimensions() {
    let previous_inputs = FullStyleWorldSnapshot::default();

    let mut next_inputs = previous_inputs.clone();
    next_inputs.environment = StyloStyleEnvironment::from_emulated_media(
        &crate::protocol_types::EmulatedMediaOverrides {
            media: Some("print".to_owned()),
            ..Default::default()
        },
    );
    next_inputs.quirks_mode = style::context::QuirksMode::Quirks;
    let previous_key = StyleWorldKey::new(
        &previous_inputs,
        StyleViewport::new(Some(800.0), Some(600.0)).with_screen_size(Some(1920.0), Some(1080.0)),
    );
    let next_key = StyleWorldKey::new(
        &next_inputs,
        StyleViewport::new(Some(1024.0), Some(768.0)).with_screen_size(Some(1366.0), Some(768.0)),
    );

    let trace = previous_key.mismatch_trace(&next_key);

    assert!(trace.viewport_changed);
    assert!(trace.screen_changed);
    assert!(trace.environment_changed);
    assert!(trace.quirks_mode_changed);
    assert!(trace.requires_style_system_replacement());
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
    let mut inputs = FullStyleWorldSnapshot {
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
        .push(CssCustomPropertyRegistrationRecord {
            registration: CssCustomPropertyRegistration {
                name: "--accent".to_owned(),
                syntax: "<color>".to_owned(),
                inherits: true,
                initial_value: Some("blue".to_owned()),
            },
            base_url: document_url.clone(),
        });

    let trace = super::super::world_trace::style_source_input_trace_for_test(&inputs);

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
    assert_eq!(trace.script_custom_property_base_urls, [document_url]);
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
    let mut standards_inputs = FullStyleWorldSnapshot::default();
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
fn document_stylesheet_fallback_updates_the_persistent_world_in_place() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, target));
    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".target { color: rgb(1, 2, 3); }".into(),
            document_url.clone(),
        ));
    let first_key = StyleWorldKey::new_for_observation(
        &first_inputs,
        StyleViewport::default(),
        StyleTreeScopeVersions::current(&host, Some(document)),
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
    assert!(engine.retained_style_system_matches_for_document_for_test(document, &first_key));
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);
    let rebuilds = engine.retained_style_system_rebuild_count_for_document_for_test(document);
    let updates = engine.retained_style_system_update_count_for_document_for_test(document);
    let cache_entries = engine.computed_style_cache_entry_count_for_document_for_test(document);

    engine.mark_document_stylesheet_set_dirty(document);

    assert!(engine.retained_style_system_matches_for_document_for_test(document, &first_key));
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "marking a source set dirty must not eagerly replace the Stylist"
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        cache_entries,
        "the last published style remains readable until the next observation"
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::DocumentStyleSheets]
    );

    let mut second_inputs = FullStyleWorldSnapshot::default();
    second_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".target { color: rgb(4, 5, 6); }".into(),
            document_url.clone(),
        ));
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
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "a full-document source fallback must still update the Stylist in place"
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        updates + 1
    );
    assert!(
        engine
            .source_dirty_scope_reasons_for_document_for_test(document)
            .is_empty()
    );
}
#[test]
fn style_subtree_invalidation_retains_style_system() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("section");
    assert!(host.append_child(document, target));
    let mut engine = MoliStyleEngine::new();
    let inputs = FullStyleWorldSnapshot::default();
    let key = StyleWorldKey::new(&inputs, None);

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
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((open_root, Vec::new()));
    inputs
        .shadow_stylesheet_sources
        .push((closed_root, Vec::new()));
    let key = StyleWorldKey::new(&inputs, None);
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
fn document_stylesheet_change_updates_the_retained_system_in_place() {
    reset_author_source_text_parse_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    let inherited_child = host.create_element("span");
    let unrelated = host.create_element("p");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "unrelated"));
    assert!(host.append_child(target, inherited_child));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/incremental.html").unwrap();
    let source_id = StyleSourceId::document_adopted_style_sheet(document, 41);
    let unrelated_source_id = StyleSourceId::document_adopted_style_sheet(document, 42);
    let unrelated_source = StyloStylesheetSource::new(
        ".unrelated { background-color: rgb(7, 8, 9); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(unrelated_source_id));
    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ".target { color: rgb(1, 2, 3); }".into(),
            document_url.clone(),
        )
        .with_source_id(Some(source_id.clone())),
    );
    first_inputs
        .document_stylesheet_sources
        .push(unrelated_source.clone());
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
        engine.computed_style_property_value(
            &host,
            &document_url,
            unrelated,
            "background-color",
            None,
            &first_inputs,
            None,
        ),
        Some("rgb(7, 8, 9)".into())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            inherited_child,
            "color",
            None,
            &first_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    let target_style_before = retained_primary_style_for_test(&engine, &host, target)
        .expect("target style should be retained");
    let unrelated_style_before = retained_primary_style_for_test(&engine, &host, unrelated)
        .expect("unrelated style should be retained");
    let inherited_style_before = retained_primary_style_for_test(&engine, &host, inherited_child)
        .expect("inherited child style should be retained");
    assert_eq!(author_source_text_parse_count_for_test(), 2);
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);
    let stylist_flushes = engine.retained_stylist_flush_count_for_document_for_test(document);
    let element_resolutions = engine.element_style_resolution_count_for_document_for_test(document);
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        0
    );

    let mut second_inputs = FullStyleWorldSnapshot::default();
    second_inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ".target { color: rgb(4, 5, 6); }".into(),
            document_url.clone(),
        )
        .with_source_id(Some(source_id)),
    );
    second_inputs
        .document_stylesheet_sources
        .push(unrelated_source);
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
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    let target_style_after = retained_primary_style_for_test(&engine, &host, target)
        .expect("target style should be recomputed");
    let unrelated_style_after = retained_primary_style_for_test(&engine, &host, unrelated)
        .expect("unrelated style should remain retained");
    assert!(!ServoArc::ptr_eq(&target_style_before, &target_style_after));
    assert!(
        ServoArc::ptr_eq(&unrelated_style_before, &unrelated_style_after),
        "Stylo stylesheet invalidation must preserve an unrelated source's canonical style"
    );
    assert!(
        !element_style_is_dirty_for_test(&engine, &host, inherited_child),
        "the inherited child remains published; its dirty-root generation is consumed on demand"
    );
    assert!(
        engine
            .computed_style_cache_contains_handle_for_document_for_test(document, inherited_child),
        "a target read must not enumerate and evict its unobserved descendants"
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            inherited_child,
            "color",
            None,
            &second_inputs,
            None,
        ),
        Some("rgb(4, 5, 6)".into()),
        "a matched ancestor invalidation must propagate to demanded inherited descendants"
    );
    let inherited_style_after = retained_primary_style_for_test(&engine, &host, inherited_child)
        .expect("inherited child should be recomputed on demand");
    assert!(!ServoArc::ptr_eq(
        &inherited_style_before,
        &inherited_style_after
    ));
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "a stylesheet revision must preserve the exact Stylist identity"
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        stylist_flushes + 1,
        "one stylesheet revision must flush the document Stylist once"
    );
    assert!(
        engine.element_style_resolution_count_for_document_for_test(document) > element_resolutions,
        "reading the invalidated target must perform a new element style resolution"
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        1,
        "a stylesheet revision must not replace the document Stylist"
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        1
    );
    let flushes_after_revision =
        engine.retained_stylist_flush_count_for_document_for_test(document);
    let resolutions_after_revision =
        engine.element_style_resolution_count_for_document_for_test(document);

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
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        1,
        "a clean read must not flush the retained style world again"
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        flushes_after_revision,
        "a clean read must not flush Stylo"
    );
    assert_eq!(
        engine.element_style_resolution_count_for_document_for_test(document),
        resolutions_after_revision,
        "a clean read must reuse the canonical ElementData style"
    );
}

#[test]
fn retained_stylesheet_resource_manifest_advances_only_when_resources_change() {
    reset_author_source_text_parse_count_for_test();
    reset_stylesheet_resource_manifest_build_count_for_test();
    let host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/resources.html").unwrap();
    let source_id = StyleSourceId::document_adopted_style_sheet(document, 71);
    let first_source = StyloStylesheetSource::new(
        "@import url(theme-a.css); @font-face { font-family: First; src: url(font-a.woff2); font-weight: 700; }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(source_id.clone()));
    engine.set_document_adopted_style_sheet_sources(document, vec![first_source.clone()]);
    let first_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![first_source],
        ..Default::default()
    };
    let first_key = StyleWorldKey::new(&first_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, first_key, &first_inputs);

    let first = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("the retained style world must publish its resource manifest");
    assert_eq!(first.web_fonts().len(), 1);
    assert_eq!(
        first.imports(),
        [url::Url::parse("https://example.test/theme-a.css").unwrap()]
    );
    assert_eq!(
        first.web_fonts()[0].request_url().as_str(),
        "https://example.test/font-a.woff2"
    );
    assert_eq!(author_source_text_parse_count_for_test(), 1);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 1);

    let clean = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("a clean world must retain its resource manifest");
    assert_eq!(clean.generation(), first.generation());
    assert_eq!(author_source_text_parse_count_for_test(), 1);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 1);

    let viewport = StyleViewport::from_width(Some(640.0));
    let viewport_key = StyleWorldKey::new(&first_inputs, viewport);
    engine.ensure_retained_style_system_for_document(&host, document, viewport_key, &first_inputs);
    let after_viewport_change = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("a device update must retain its resource manifest");
    assert_eq!(
        after_viewport_change.generation(),
        first.generation(),
        "device-only style updates must not advance the resource revision"
    );
    assert_eq!(
        stylesheet_resource_manifest_build_count_for_test(),
        2,
        "a device update must reproject effective resources without advancing an unchanged manifest"
    );

    let same_resources_source = StyloStylesheetSource::new(
        "@import url(theme-a.css); @font-face { font-family: First; src: url(font-a.woff2); font-weight: 700; } body { color: green; }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(source_id.clone()));
    engine.set_document_adopted_style_sheet_sources(document, vec![same_resources_source.clone()]);
    let same_resources_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![same_resources_source],
        ..Default::default()
    };
    let same_resources_key = StyleWorldKey::new(&same_resources_inputs, viewport);
    engine.ensure_retained_style_system_for_document(
        &host,
        document,
        same_resources_key,
        &same_resources_inputs,
    );
    let after_non_resource_change = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("a non-resource rule update must retain its resource manifest");
    assert_eq!(
        after_non_resource_change.generation(),
        first.generation(),
        "ordinary declaration changes must not restart resource reconciliation"
    );
    assert_eq!(author_source_text_parse_count_for_test(), 2);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 3);

    let second_source = StyloStylesheetSource::new(
        "@import url(theme-b.css); @font-face { font-family: Second; src: url(font-b.woff2); font-style: italic; }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(source_id));
    engine.set_document_adopted_style_sheet_sources(document, vec![second_source.clone()]);
    let second_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![second_source],
        ..Default::default()
    };
    let second_key = StyleWorldKey::new(&second_inputs, viewport);
    engine.ensure_retained_style_system_for_document(&host, document, second_key, &second_inputs);

    let second = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("the revised world must publish its new resource manifest");
    assert_ne!(second.generation(), first.generation());
    assert_eq!(second.web_fonts().len(), 1);
    assert_eq!(
        second.imports(),
        [url::Url::parse("https://example.test/theme-b.css").unwrap()]
    );
    assert_eq!(
        second.web_fonts()[0].request_url().as_str(),
        "https://example.test/font-b.woff2"
    );
    assert_eq!(
        author_source_text_parse_count_for_test(),
        3,
        "each stylesheet revision must be parsed once for both cascade and resources"
    );
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 4);
}

#[test]
fn retained_imported_font_resources_keep_each_import_parser_base_and_response_slot() {
    use style::{context::QuirksMode, stylesheets::AllowImportRules};

    let host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let registry = crate::live_stylesheet::LiveStylesheetRegistry::default();
    let root = registry.create(
        concat!(
            "@import '../theme/first/imported.css';",
            "@import '../theme/second/imported.css';",
            "@import '../redirect/entry.css';",
        ),
        url::Url::parse("https://example.test/css/root.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        engine.author_shared_lock(),
    );
    let responses = vec![
        crate::live_stylesheet::LiveStylesheetImportResponse {
            request_url: url::Url::parse("https://example.test/theme/first/imported.css").unwrap(),
            response_url: url::Url::parse("https://example.test/theme/first/imported.css").unwrap(),
            css_text: concat!(
                "@font-face { font-family: FirstImported; ",
                "src: url('./fonts/shared.woff2') format('woff2'); }",
            )
            .to_owned(),
            successful: true,
            origin_clean: true,
        },
        crate::live_stylesheet::LiveStylesheetImportResponse {
            request_url: url::Url::parse("https://example.test/theme/second/imported.css").unwrap(),
            response_url: url::Url::parse("https://example.test/theme/second/imported.css")
                .unwrap(),
            css_text: concat!(
                "@font-face { font-family: SecondImported; ",
                "src: url('./fonts/shared.woff2') format('woff2'); }",
            )
            .to_owned(),
            successful: true,
            origin_clean: true,
        },
        crate::live_stylesheet::LiveStylesheetImportResponse {
            request_url: url::Url::parse("https://example.test/redirect/entry.css").unwrap(),
            response_url: url::Url::parse("https://cdn.example.test/final/entry.css").unwrap(),
            css_text: "@import './nested.css';".to_owned(),
            successful: true,
            origin_clean: true,
        },
        crate::live_stylesheet::LiveStylesheetImportResponse {
            request_url: url::Url::parse("https://cdn.example.test/final/nested.css").unwrap(),
            response_url: url::Url::parse("https://assets.example.test/styles/nested.css").unwrap(),
            css_text: concat!(
                "@font-face { font-family: RedirectedNested; ",
                "src: local('RedirectedNested'), ",
                "url('../fonts/nested.svg') format('svg'), ",
                "url('../fonts/nested.woff2') format('woff2'); }",
            )
            .to_owned(),
            successful: true,
            origin_clean: true,
        },
    ];
    assert_eq!(
        registry.install_import_graph(
            root.id(),
            root.contents_revision(),
            root.import_generation(),
            &responses,
            Some(root.base_url()),
        ),
        Some(true),
        "the nested redirected import graph should install completely",
    );

    let source = StyloStylesheetSource::from_live_stylesheet(&root).with_source_id(Some(
        StyleSourceId::document_adopted_style_sheet(document, 73),
    ));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![source],
        ..Default::default()
    };
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);

    let snapshot = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("the imported font graph should publish a resource manifest");
    let retained_by_url = snapshot
        .web_fonts()
        .iter()
        .map(|resource| {
            (
                resource.request_url().as_str().to_owned(),
                resource
                    .web_font()
                    .expect("font manifest entry")
                    .slot()
                    .to_owned(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        retained_by_url.keys().cloned().collect::<Vec<_>>(),
        [
            "https://assets.example.test/fonts/nested.woff2".to_owned(),
            "https://example.test/theme/first/fonts/shared.woff2".to_owned(),
            "https://example.test/theme/second/fonts/shared.woff2".to_owned(),
        ],
        "every imported rule must retain its own response/parser base, including redirects",
    );
    assert_ne!(
        retained_by_url["https://example.test/theme/first/fonts/shared.woff2"],
        retained_by_url["https://example.test/theme/second/fonts/shared.woff2"],
        "the same relative src in different imported directories needs distinct slots",
    );

    for response in responses {
        for early in crate::css_resource_urls::stylesheet_load_blocking_font_resources(
            &response.css_text,
            &response.response_url,
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        ) {
            let retained_slot = &retained_by_url[early.request_url().as_str()];
            assert_eq!(
                retained_slot,
                early.web_font().expect("early response font").slot(),
                "response-time registration and retained reconciliation must share a slot",
            );
        }
    }
}

#[test]
fn retained_stylesheet_resource_manifest_tracks_effective_font_faces_across_media_changes() {
    reset_author_source_text_parse_count_for_test();
    reset_stylesheet_resource_manifest_build_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    assert!(host.append_child(document, shadow_host));
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("font resource fixture should create a ShadowRoot");
    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/media-fonts.html").unwrap();
    let owner_media_source = StyloStylesheetSource::new(
        "@font-face { font-family: OwnerPrint; src: url(owner-print.woff2); }".into(),
        document_url.clone(),
    )
    .with_owner_media_text("print");
    let nested_media_source = StyloStylesheetSource::new(
        "@media print { @font-face { font-family: NestedPrint; src: url(nested-print.woff2); } }\
         @media screen { @font-face { font-family: ScreenOnly; src: url(screen.woff2); } }"
            .into(),
        document_url,
    );
    let mut screen_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![owner_media_source, nested_media_source],
        ..Default::default()
    };
    screen_inputs.shadow_stylesheet_sources.push((
        shadow_root,
        vec![StyloStylesheetSource::new(
            "@media print { @font-face { font-family: ShadowPrint; src: url(shadow-print.woff2); } }\
             @media screen { @font-face { font-family: ShadowScreen; src: url(shadow-screen.woff2); } }"
                .into(),
            url::Url::parse("https://example.test/media-fonts.html").unwrap(),
        )],
    ));
    let screen_key = StyleWorldKey::new(&screen_inputs, None);
    engine.ensure_retained_style_system_for_document(
        &host,
        document,
        screen_key.clone(),
        &screen_inputs,
    );

    let resource_urls = |snapshot: &StylesheetResourceSnapshot| {
        let mut urls = snapshot
            .web_fonts()
            .iter()
            .map(|resource| resource.request_url().as_str().to_owned())
            .collect::<Vec<_>>();
        urls.sort();
        urls
    };
    let screen = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("screen media must publish an effective font projection");
    assert_eq!(
        resource_urls(&screen),
        [
            "https://example.test/screen.woff2",
            "https://example.test/shadow-screen.woff2",
        ]
    );
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 1);
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);

    let mut print_inputs = screen_inputs.clone();
    print_inputs.environment = StyloStyleEnvironment::from_emulated_media(
        &crate::protocol_types::EmulatedMediaOverrides {
            media: Some("print".to_owned()),
            ..Default::default()
        },
    );
    let print_key = StyleWorldKey::new(&print_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, print_key, &print_inputs);
    let print = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("print media must publish its effective font projection");
    assert_ne!(print.generation(), screen.generation());
    assert_eq!(
        resource_urls(&print),
        [
            "https://example.test/nested-print.woff2",
            "https://example.test/owner-print.woff2",
            "https://example.test/shadow-print.woff2",
        ]
    );
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "media changes must update the retained Stylist in place"
    );
    assert_eq!(
        author_source_text_parse_count_for_test(),
        3,
        "device projection must reuse parsed native rules"
    );

    engine.ensure_retained_style_system_for_document(&host, document, screen_key, &screen_inputs);
    let restored_screen = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("returning to screen must publish the restored projection");
    assert_ne!(restored_screen.generation(), print.generation());
    assert_eq!(
        resource_urls(&restored_screen),
        [
            "https://example.test/screen.woff2",
            "https://example.test/shadow-screen.woff2",
        ]
    );
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity
    );
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 3);
}

#[test]
fn document_stylesheet_append_and_reorder_reuse_parsed_sheets() {
    reset_author_source_text_parse_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, target));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/ordered-sheets.html").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
        document, 61,
    )));
    let second_source = StyloStylesheetSource::new(
        ".target { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
        document, 62,
    )));

    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs
        .document_stylesheet_sources
        .push(first_source.clone());
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
    assert_eq!(author_source_text_parse_count_for_test(), 1);

    let appended_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![first_source.clone(), second_source.clone()],
        ..Default::default()
    };
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &appended_inputs,
            None,
        ),
        Some("rgb(4, 5, 6)".into())
    );
    assert_eq!(
        author_source_text_parse_count_for_test(),
        2,
        "appending one sheet must not reparse the existing sheet"
    );

    let reordered_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![second_source, first_source],
        ..Default::default()
    };
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &reordered_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        author_source_text_parse_count_for_test(),
        2,
        "reordering sheets must reuse both parsed stylesheet objects"
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        2
    );
}

#[test]
fn incremental_document_stylesheet_updates_match_a_fresh_style_world_oracle() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("section");
    let child = host.create_element("span");
    let unrelated = host.create_element("aside");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "unrelated"));
    assert!(host.append_child(target, child));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let document_url = url::Url::parse("https://example.test/oracle.html").unwrap();
    let first_id = StyleSourceId::document_adopted_style_sheet(document, 81);
    let second_id = StyleSourceId::document_adopted_style_sheet(document, 82);
    let source = |css: &str, id: StyleSourceId| {
        StyloStylesheetSource::new(css.into(), document_url.clone()).with_source_id(Some(id))
    };
    let first = source(
        ".target { color: rgb(1, 2, 3); background-color: rgb(4, 5, 6); }",
        first_id.clone(),
    );
    let first_revision = source(
        ".target { color: rgb(7, 8, 9); background-color: rgb(10, 11, 12); }",
        first_id,
    );
    let second = source(
        ".target { color: rgb(13, 14, 15); } .unrelated { color: rgb(16, 17, 18); }",
        second_id,
    );
    let sequences = [
        vec![first.clone()],
        vec![first.clone(), second.clone()],
        vec![first_revision.clone(), second.clone()],
        vec![second.clone(), first_revision.clone()],
        vec![first_revision],
        Vec::new(),
    ];

    let mut incremental = MoliStyleEngine::new();
    for sources in sequences {
        incremental.set_document_adopted_style_sheet_sources(document, sources.clone());
        let inputs = FullStyleWorldSnapshot {
            document_stylesheet_sources: sources,
            ..Default::default()
        };
        let oracle = MoliStyleEngine::new();
        for (element, property) in [
            (target, "color"),
            (target, "background-color"),
            (child, "color"),
            (unrelated, "color"),
        ] {
            assert_eq!(
                incremental.computed_style_property_value(
                    &host,
                    &document_url,
                    element,
                    property,
                    None,
                    &inputs,
                    None,
                ),
                oracle.computed_style_property_value(
                    &host,
                    &document_url,
                    element,
                    property,
                    None,
                    &inputs,
                    None,
                ),
                "incremental and fresh worlds diverged for {element:?} {property}"
            );
        }
    }

    assert_eq!(
        incremental.retained_style_system_rebuild_count_for_document_for_test(document),
        1,
        "the oracle sequence must keep one persistent document Stylist"
    );
    assert_eq!(
        incremental.retained_style_system_update_count_for_document_for_test(document),
        5
    );
}

#[test]
fn one_shadow_stylesheet_change_preserves_the_other_scope_cascade_data() {
    reset_author_source_text_parse_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let first_host = host.create_element("section");
    let second_host = host.create_element("article");
    assert!(host.append_child(document, first_host));
    assert!(host.append_child(document, second_host));
    let first_root = host
        .attach_shadow_root(first_host, "open")
        .expect("first host should accept a shadow root");
    let second_root = host
        .attach_shadow_root(second_host, "open")
        .expect("second host should accept a shadow root");

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/shadow-incremental.html").unwrap();
    let first_source_id = StyleSourceId::shadow_root_adopted_style_sheet(first_root, 51);
    let second_source_id = StyleSourceId::shadow_root_adopted_style_sheet(second_root, 52);
    let first_source = StyloStylesheetSource::new(
        ":host { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(first_source_id.clone()));
    let second_source = StyloStylesheetSource::new(
        ":host { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(second_source_id));
    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs
        .shadow_stylesheet_sources
        .push((first_root, vec![first_source]));
    first_inputs
        .shadow_stylesheet_sources
        .push((second_root, vec![second_source.clone()]));
    let first_key = StyleWorldKey::new(&first_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, first_key, &first_inputs);

    let initial_data =
        engine.with_retained_style_system_for_document_for_test(document, |retained| {
            retained
                .shadow_cascade_data
                .iter()
                .map(|(root, data)| (*root, data.clone()))
                .collect::<Vec<_>>()
        });
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);
    let first_scope_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, first_root)
        .expect("first scope should be retained");
    let second_scope_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, second_root)
        .expect("second scope should be retained");
    assert_eq!(author_source_text_parse_count_for_test(), 2);

    let revised_first_source = StyloStylesheetSource::new(
        ":host { color: rgb(7, 8, 9); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(first_source_id));
    let mut second_inputs = FullStyleWorldSnapshot::default();
    second_inputs
        .shadow_stylesheet_sources
        .push((first_root, vec![revised_first_source]));
    second_inputs
        .shadow_stylesheet_sources
        .push((second_root, vec![second_source]));
    let second_key = StyleWorldKey::new(&second_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, second_key, &second_inputs);

    engine.with_retained_style_system_for_document_for_test(document, |retained| {
        let current_data = retained
            .shadow_cascade_data
            .iter()
            .map(|(root, data)| (*root, data))
            .collect::<Vec<_>>();
        assert!(!ServoArc::ptr_eq(&initial_data[0].1, current_data[0].1));
        assert!(ServoArc::ptr_eq(&initial_data[1].1, current_data[1].1));
    });
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, first_root),
        Some(first_scope_flushes + 1),
        "the dirty ShadowRoot must flush exactly once"
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, second_root),
        Some(second_scope_flushes),
        "an unrelated ShadowRoot must not flush"
    );
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        1
    );
}

#[test]
fn viewport_resize_recascades_cached_viewport_units_without_media_match_changes() {
    reset_source_cascade_rebuild_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let rule_target = host.create_element("div");
    assert!(host.set_attribute(rule_target, "class", "rule-target stable-media-target"));
    assert!(host.append_child(document, rule_target));
    let inline_target = host.create_element("div");
    assert!(host.set_attribute(inline_target, "style", "width: 12vw; height: 25vh",));
    assert!(host.append_child(document, inline_target));
    let shadow_host = host.create_element("section");
    assert!(host.append_child(document, shadow_host));
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("the viewport-unit host should accept a shadow root");
    let shadow_target = host.create_element("span");
    assert!(host.set_attribute(shadow_target, "class", "shadow-target"));
    assert!(host.append_child(shadow_root, shadow_target));

    let document_url = url::Url::parse("https://example.test/viewport-units.html").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            r#"
              .rule-target {
                position: absolute;
                width: 10vw;
                height: 10vh;
                min-width: 2vmin;
                max-width: 2vmax;
                font-size: 2vmin;
                padding-left: calc(5vw + 8px);
                top: 10dvh;
                right: 10svh;
                bottom: 10lvh;
              }
              @media (min-width: 1px) {
                .stable-media-target { left: 50vw; }
              }
            "#
            .into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 81,
        ))),
    );
    inputs.shadow_stylesheet_sources.push((
        shadow_root,
        vec![
            StyloStylesheetSource::new(
                ".shadow-target { width: 25vw; height: 25vh; }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(StyleSourceId::shadow_root_adopted_style_sheet(
                shadow_root,
                82,
            ))),
        ],
    ));

    let engine = MoliStyleEngine::new();
    let viewport_1000 = StyleViewport::new(Some(1000.0), Some(800.0));
    let read = |target, property, viewport| {
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
                property,
                None,
                &inputs,
                viewport,
            )
            .unwrap_or_else(|| panic!("{property} should have a computed value"))
    };
    let rule_properties = [
        "width",
        "height",
        "min-width",
        "max-width",
        "font-size",
        "padding-left",
        "top",
        "right",
        "bottom",
        "left",
    ];
    assert_eq!(
        rule_properties.map(|property| read(rule_target, property, viewport_1000)),
        [
            "100px", "80px", "16px", "20px", "16px", "58px", "80px", "80px", "80px", "500px",
        ],
    );
    assert_eq!(read(inline_target, "width", viewport_1000), "120px");
    assert_eq!(read(inline_target, "height", viewport_1000), "200px");
    assert_eq!(read(shadow_target, "width", viewport_1000), "250px");
    assert_eq!(read(shadow_target, "height", viewport_1000), "200px");
    let style_before = retained_primary_style_for_test(&engine, &host, rule_target)
        .expect("the initial viewport-relative style should be retained");
    let document_flushes = engine.retained_stylist_flush_count_for_document_for_test(document);
    let shadow_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, shadow_root)
        .expect("the shadow scope should be retained");
    let source_rebuilds = source_cascade_rebuild_count_for_test();

    let viewport_500 = StyleViewport::new(Some(500.0), Some(400.0));
    assert_eq!(
        rule_properties.map(|property| read(rule_target, property, viewport_500)),
        [
            "50px", "40px", "8px", "10px", "8px", "33px", "40px", "40px", "40px", "250px",
        ],
        "the same cached element must recascade every viewport-relative unit",
    );
    assert_eq!(read(inline_target, "width", viewport_500), "60px");
    assert_eq!(read(inline_target, "height", viewport_500), "100px");
    assert_eq!(read(shadow_target, "width", viewport_500), "125px");
    assert_eq!(read(shadow_target, "height", viewport_500), "100px");
    let style_after = retained_primary_style_for_test(&engine, &host, rule_target)
        .expect("the resized viewport-relative style should be retained");
    assert!(
        !ServoArc::ptr_eq(&style_before, &style_after),
        "viewport invalidation must replace the canonical ComputedValues on the same element",
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        document_flushes,
        "an always-matching media query must not be used to hide viewport-unit invalidation",
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, shadow_root),
        Some(shadow_flushes),
        "viewport units alone must not flush the ShadowRoot AuthorStyles",
    );
    assert_eq!(
        source_cascade_rebuild_count_for_test(),
        source_rebuilds,
        "viewport units alone must not rebuild source-local cascade data",
    );
}

#[test]
fn repeated_viewport_resizes_recascade_only_dependent_cached_elements() {
    let mut host = test_host();
    let document = host.document_handle();
    let dependent = host.create_element("div");
    let independent = host.create_element("div");
    assert!(host.set_attribute(dependent, "class", "viewport-dependent"));
    assert!(host.set_attribute(independent, "class", "viewport-independent"));
    assert!(host.append_child(document, dependent));
    assert!(host.append_child(document, independent));

    let document_url = url::Url::parse("https://example.test/repeated-resize.html").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            r#"
              .viewport-dependent { width: 10vw; height: 10vh; }
              .viewport-independent { width: 123px; height: 45px; }
            "#
            .into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 83,
        ))),
    );

    let engine = MoliStyleEngine::new();
    let read = |target, property, viewport| {
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
                property,
                None,
                &inputs,
                viewport,
            )
            .unwrap_or_else(|| panic!("{property} should have a computed value"))
    };
    let large = StyleViewport::new(Some(800.0), Some(600.0));
    assert_eq!(read(dependent, "width", large), "80px");
    assert_eq!(read(independent, "width", large), "123px");
    let first_dependent = retained_primary_style_for_test(&engine, &host, dependent).unwrap();
    let first_independent = retained_primary_style_for_test(&engine, &host, independent).unwrap();
    let flushes = engine.retained_stylist_flush_count_for_document_for_test(document);

    let small = StyleViewport::new(Some(400.0), Some(300.0));
    assert_eq!(read(dependent, "width", small), "40px");
    assert_eq!(read(dependent, "height", small), "30px");
    assert_eq!(read(independent, "width", small), "123px");
    let second_dependent = retained_primary_style_for_test(&engine, &host, dependent).unwrap();
    let second_independent = retained_primary_style_for_test(&engine, &host, independent).unwrap();
    assert!(!ServoArc::ptr_eq(&first_dependent, &second_dependent));
    assert!(
        ServoArc::ptr_eq(&first_independent, &second_independent),
        "a viewport-independent cached element must survive a resize unchanged",
    );

    assert_eq!(read(dependent, "width", large), "80px");
    assert_eq!(read(dependent, "height", large), "60px");
    assert_eq!(read(independent, "height", large), "45px");
    let third_dependent = retained_primary_style_for_test(&engine, &host, dependent).unwrap();
    let third_independent = retained_primary_style_for_test(&engine, &host, independent).unwrap();
    assert!(!ServoArc::ptr_eq(&second_dependent, &third_dependent));
    assert!(
        ServoArc::ptr_eq(&second_independent, &third_independent),
        "reversing a resize must still preserve viewport-independent ComputedValues",
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        flushes,
        "viewport-unit recascade must not flush an unchanged stylesheet",
    );
}

#[test]
fn viewport_resize_propagates_through_inheritance_variables_pseudos_and_nested_shadow_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let child = host.create_element("span");
    let pseudo_target = host.create_element("div");
    assert!(host.set_attribute(parent, "class", "viewport-parent"));
    assert!(host.set_attribute(child, "class", "viewport-child"));
    assert!(host.set_attribute(pseudo_target, "class", "viewport-pseudo"));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, child));
    assert!(host.append_child(document, pseudo_target));

    let outer_host = host.create_element("article");
    assert!(host.append_child(document, outer_host));
    let outer_root = host
        .attach_shadow_root(outer_host, "open")
        .expect("outer host should accept a shadow root");
    let outer_target = host.create_element("div");
    let inner_host = host.create_element("aside");
    assert!(host.set_attribute(outer_target, "class", "outer-viewport-target"));
    assert!(host.append_child(outer_root, outer_target));
    assert!(host.append_child(outer_root, inner_host));
    let inner_root = host
        .attach_shadow_root(inner_host, "open")
        .expect("a host inside a ShadowRoot should accept a nested shadow root");
    let inner_target = host.create_element("span");
    assert!(host.set_attribute(inner_target, "class", "inner-viewport-target"));
    assert!(host.append_child(inner_root, inner_target));

    let document_url = url::Url::parse("https://example.test/viewport-dependencies.html").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            r#"
              .viewport-parent { font-size: 2vmin; --viewport-gap: 10vw; }
              .viewport-child { font-size: 2em; padding-left: var(--viewport-gap); }
              .viewport-pseudo::before {
                content: "viewport";
                width: 10vw;
                height: 10vh;
              }
            "#
            .into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 84,
        ))),
    );
    inputs.shadow_stylesheet_sources.push((
        outer_root,
        vec![
            StyloStylesheetSource::new(
                ".outer-viewport-target { width: 20vw; }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(StyleSourceId::shadow_root_adopted_style_sheet(
                outer_root, 85,
            ))),
        ],
    ));
    inputs.shadow_stylesheet_sources.push((
        inner_root,
        vec![
            StyloStylesheetSource::new(
                ".inner-viewport-target { height: 15vh; }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(StyleSourceId::shadow_root_adopted_style_sheet(
                inner_root, 86,
            ))),
        ],
    ));

    let engine = MoliStyleEngine::new();
    let read = |target, property, pseudo, viewport| {
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
                property,
                pseudo,
                &inputs,
                viewport,
            )
            .unwrap_or_else(|| panic!("{property} should have a computed value"))
    };
    let large = StyleViewport::new(Some(1000.0), Some(800.0));
    assert_eq!(read(parent, "font-size", None, large), "16px");
    assert_eq!(read(child, "font-size", None, large), "32px");
    assert_eq!(read(child, "padding-left", None, large), "100px");
    assert_eq!(read(pseudo_target, "width", Some("before"), large), "100px");
    assert_eq!(read(pseudo_target, "height", Some("before"), large), "80px");
    assert_eq!(read(outer_target, "width", None, large), "200px");
    assert_eq!(read(inner_target, "height", None, large), "120px");
    let child_before = retained_primary_style_for_test(&engine, &host, child).unwrap();
    let nested_before = retained_primary_style_for_test(&engine, &host, inner_target).unwrap();
    let outer_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, outer_root)
        .expect("outer shadow scope should be retained");
    let inner_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, inner_root)
        .expect("inner shadow scope should be retained");

    let small = StyleViewport::new(Some(500.0), Some(400.0));
    assert_eq!(read(parent, "font-size", None, small), "8px");
    assert_eq!(
        read(child, "font-size", None, small),
        "16px",
        "viewport invalidation must propagate through inherited font metrics",
    );
    assert_eq!(
        read(child, "padding-left", None, small),
        "50px",
        "a viewport unit substituted through an inherited custom property must recascade",
    );
    assert_eq!(read(pseudo_target, "width", Some("before"), small), "50px");
    assert_eq!(read(pseudo_target, "height", Some("before"), small), "40px");
    assert_eq!(read(outer_target, "width", None, small), "100px");
    assert_eq!(read(inner_target, "height", None, small), "60px");
    let child_after = retained_primary_style_for_test(&engine, &host, child).unwrap();
    let nested_after = retained_primary_style_for_test(&engine, &host, inner_target).unwrap();
    assert!(!ServoArc::ptr_eq(&child_before, &child_after));
    assert!(!ServoArc::ptr_eq(&nested_before, &nested_after));
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, outer_root),
        Some(outer_flushes),
        "viewport dependency invalidation must not rebuild the outer AuthorStyles",
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, inner_root),
        Some(inner_flushes),
        "viewport dependency invalidation must not rebuild nested AuthorStyles",
    );
}

#[test]
fn device_changes_keep_media_sheets_installed_and_flush_only_affected_tree_scopes() {
    reset_source_cascade_rebuild_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let document_target = host.create_element("div");
    assert!(host.set_attribute(document_target, "class", "document-target"));
    assert!(host.set_attribute(document_target, "style", "width: 50vw"));
    assert!(host.append_child(document, document_target));
    let first_host = host.create_element("section");
    let second_host = host.create_element("article");
    assert!(host.append_child(document, first_host));
    assert!(host.append_child(document, second_host));
    let first_root = host
        .attach_shadow_root(first_host, "open")
        .expect("first host should accept a shadow root");
    let second_root = host
        .attach_shadow_root(second_host, "open")
        .expect("second host should accept a shadow root");
    let first_target = host.create_element("span");
    let second_target = host.create_element("span");
    assert!(host.set_attribute(first_target, "class", "first-target"));
    assert!(host.set_attribute(first_target, "style", "height: 50vh"));
    assert!(host.set_attribute(second_target, "class", "second-target"));
    assert!(host.append_child(first_root, first_target));
    assert!(host.append_child(second_root, second_target));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/shadow-media.html").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ".document-target { color: rgb(7, 8, 9); }".into(),
            document_url.clone(),
        )
        .with_owner_media_text("(max-width: 600px)"),
    );
    inputs.shadow_stylesheet_sources.push((
        first_root,
        vec![
            StyloStylesheetSource::new(
                ".first-target { color: rgb(1, 2, 3); }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(StyleSourceId::shadow_root_adopted_style_sheet(
                first_root, 71,
            )))
            .with_owner_media_text("(max-width: 600px)"),
        ],
    ));
    inputs.shadow_stylesheet_sources.push((
        second_root,
        vec![
            StyloStylesheetSource::new(
                ".second-target { color: rgb(4, 5, 6); }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(StyleSourceId::shadow_root_adopted_style_sheet(
                second_root,
                72,
            )))
            .with_owner_media_text("(min-width: 100px)"),
        ],
    ));

    let viewport_800 = StyleViewport::new(Some(800.0), Some(600.0));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            document_target,
            "color",
            None,
            &inputs,
            viewport_800,
        ),
        Some("rgb(0, 0, 0)".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "color",
            None,
            &inputs,
            viewport_800,
        ),
        Some("rgb(0, 0, 0)".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            second_target,
            "color",
            None,
            &inputs,
            viewport_800,
        ),
        Some("rgb(4, 5, 6)".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            document_target,
            "width",
            None,
            &inputs,
            viewport_800,
        ),
        Some("400px".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "height",
            None,
            &inputs,
            viewport_800,
        ),
        Some("300px".into()),
    );
    engine.with_retained_style_system_for_document_for_test(document, |retained| {
        assert_eq!(
            retained.document_stylesheets.entries().len(),
            1,
            "a non-matching owner MediaList must not remove its sheet from the Document scope",
        );
        assert_eq!(retained.shadow_scopes.len(), 2);
        assert!(
            retained
                .shadow_scopes
                .iter()
                .all(|scope| scope.active_stylesheets().entries().len() == 1),
            "a non-matching owner MediaList must not remove its sheet from the TreeScope",
        );
    });
    let document_flushes = engine.retained_stylist_flush_count_for_document_for_test(document);
    let first_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, first_root)
        .expect("first scope should be retained");
    let second_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, second_root)
        .expect("second scope should be retained");
    let source_rebuilds = source_cascade_rebuild_count_for_test();

    let viewport_700 = StyleViewport::new(Some(700.0), Some(500.0));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "color",
            None,
            &inputs,
            viewport_700,
        ),
        Some("rgb(0, 0, 0)".into()),
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, first_root),
        Some(first_flushes),
        "a viewport change that crosses no media boundary must not flush the first scope",
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, second_root),
        Some(second_flushes),
        "a viewport change that crosses no media boundary must not flush the second scope",
    );
    assert_eq!(source_cascade_rebuild_count_for_test(), source_rebuilds);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            document_target,
            "width",
            None,
            &inputs,
            viewport_700,
        ),
        Some("350px".into()),
        "viewport units must be invalidated without a Document stylesheet flush",
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "height",
            None,
            &inputs,
            viewport_700,
        ),
        Some("250px".into()),
        "viewport units inside ShadowRoot must be invalidated without an AuthorStyles flush",
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        document_flushes,
        "a viewport change that crosses no media boundary must not flush the Document Stylist",
    );

    let viewport_500 = StyleViewport::new(Some(500.0), Some(600.0));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            document_target,
            "color",
            None,
            &inputs,
            viewport_500,
        ),
        Some("rgb(7, 8, 9)".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "color",
            None,
            &inputs,
            viewport_500,
        ),
        Some("rgb(1, 2, 3)".into()),
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, first_root),
        Some(first_flushes + 1),
        "only the scope whose top-level MediaList changed must flush",
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, second_root),
        Some(second_flushes),
        "an unrelated matching MediaList must preserve its AuthorStyles",
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        document_flushes + 1,
        "the Document Stylist must flush once when its installed MediaList changes match state",
    );
    assert_eq!(
        source_cascade_rebuild_count_for_test(),
        source_rebuilds + 1,
        "only the affected scope's source-local cascade projection must rebuild",
    );
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
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((first_shadow_root, vec![first_source]));
    inputs
        .shadow_stylesheet_sources
        .push((second_shadow_root, vec![second_source]));
    let key = StyleWorldKey::new(&inputs, None);
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
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((first_shadow_root, vec![first_source]));
    inputs
        .shadow_stylesheet_sources
        .push((second_shadow_root, vec![second_source]));
    let key = StyleWorldKey::new(&inputs, None);
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
fn document_stylesheet_dirty_mark_isolated_to_its_document_world() {
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

    let mut active_inputs = FullStyleWorldSnapshot::default();
    active_inputs
        .shadow_stylesheet_sources
        .push((active_shadow_root, vec![active_source]));
    let active_key = StyleWorldKey::new(&active_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, active_key, &active_inputs);

    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .shadow_stylesheet_sources
        .push((detached_shadow_root, vec![detached_source]));
    let detached_key = StyleWorldKey::new(&detached_inputs, None);
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

    engine.mark_document_stylesheet_set_dirty(detached_document);

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
    assert!(engine.dom_adapter.has_element_data(active_shadow_host));
    assert!(engine.dom_adapter.has_element_data(detached_shadow_host));
    assert_eq!(
        engine.dom_adapter.shadow_cascade_document_count_for_test(),
        2
    );
    assert_only_source_document_is_dirty(&engine, detached_document, document);
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
    let inputs = FullStyleWorldSnapshot::default();
    let active_key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, active_key, &inputs);
    ensure_adapter_element_data(&engine, &host, active);
    assert!(engine.dom_adapter.has_element_data(active));

    let detached_source = StyloStylesheetSource::new(
        "section { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let mut first_detached_inputs = FullStyleWorldSnapshot::default();
    first_detached_inputs
        .document_stylesheet_sources
        .push(detached_source);
    let first_detached_key = StyleWorldKey::new(&first_detached_inputs, None);
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
    let mut next_detached_inputs = FullStyleWorldSnapshot::default();
    next_detached_inputs
        .document_stylesheet_sources
        .push(next_detached_source);
    let next_detached_key = StyleWorldKey::new(&next_detached_inputs, None);
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

fn computed_style_snapshot_for_test(
    engine: &MoliStyleEngine,
    host: &crate::dom::native::DomHost,
    document_url: &url::Url,
    handle: crate::document_runtime::DomHandle,
    inputs: &FullStyleWorldSnapshot,
) -> StyloComputedStyleSnapshot {
    let document = host
        .owner_document_handle(handle)
        .expect("test element should have an owner document");
    engine
        .computed_style_snapshot_after_style_update_with_document_context(
            host,
            document_url,
            handle,
            inputs,
            StyleSourceDocumentContext::for_root_document(document),
            document,
            StyleViewport::default(),
        )
        .expect("test element should have computed style")
}

fn retained_primary_style_for_test(
    engine: &MoliStyleEngine,
    host: &crate::dom::native::DomHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<ServoArc<style::properties::ComputedValues>> {
    engine.dom_adapter.with_bound_host(host, |adapter| {
        let element = adapter.element(host, handle)?;
        let data = element.borrow_data()?;
        data.has_styles().then(|| data.styles.primary().clone())
    })
}

fn element_style_is_dirty_for_test(
    engine: &MoliStyleEngine,
    host: &crate::dom::native::DomHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    engine.dom_adapter.with_bound_host(host, |adapter| {
        let element = adapter.element(host, handle).expect("test element");
        element
            .borrow_data()
            .is_some_and(|data| !data.hint.is_empty())
    })
}
