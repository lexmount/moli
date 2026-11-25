use super::*;

#[test]
fn retained_style_system_cache_hit_preserves_computed_and_retained_generations() {
    let engine = MoliStyleEngine::new();
    let host = test_host();
    let document = host.document_handle();
    let inputs = FullStyleWorldSnapshot::default();
    let key = StyleWorldKey::new(&inputs, None);

    engine.ensure_retained_style_system_for_document(
        &host,
        host.document_handle(),
        key.clone(),
        &inputs,
    );
    let computed_generation_after_first_build =
        engine.computed_cache_generation_for_document_for_test(document);
    let retained_generation_after_first_build =
        engine.retained_style_system_generation_for_document_for_test(document);
    engine.ensure_retained_style_system_for_document(&host, host.document_handle(), key, &inputs);

    assert_eq!(computed_generation_after_first_build, 1);
    assert_eq!(retained_generation_after_first_build, 1);
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        computed_generation_after_first_build
    );
    assert_eq!(
        engine.retained_style_system_generation_for_document_for_test(document),
        retained_generation_after_first_build
    );
}

#[test]
fn viewport_change_updates_the_persistent_style_world_in_place() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("main");
    let sibling = host.create_element("aside");
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, sibling));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![StyloStylesheetSource::new(
            "main { color: rgb(1, 2, 3); }\n\
             aside { color: rgb(7, 8, 9); }\n\
             @media (max-width: 700px) { main { color: rgb(4, 5, 6); } }"
                .to_owned(),
            document_url.clone(),
        )],
        ..Default::default()
    };
    let initial_viewport = StyleViewport::from_width(Some(800.0));
    let next_viewport = StyleViewport::from_width(Some(640.0));
    let initial_key = StyleWorldKey::new(&inputs, initial_viewport);
    let viewport_key = StyleWorldKey::new(&inputs, next_viewport);
    let mismatch = initial_key.mismatch_trace(&viewport_key);
    assert!(mismatch.viewport_changed);

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &inputs,
            initial_viewport,
        ),
        Some("rgb(1, 2, 3)".to_owned())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            sibling,
            "color",
            None,
            &inputs,
            initial_viewport,
        ),
        Some("rgb(7, 8, 9)".to_owned())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    let computed_generation_after_cached_reads =
        engine.computed_cache_generation_for_document_for_test(document);
    let source_set_generation_after_cached_reads =
        engine.source_set_generation_for_document_for_test(document);
    let retained_generation_after_cached_reads =
        engine.retained_style_system_generation_for_document_for_test(document);
    let retained_identity_after_cached_reads =
        engine.retained_stylist_identity_for_document_for_test(document);
    let retained_rebuilds_after_cached_reads =
        engine.retained_style_system_rebuild_count_for_document_for_test(document);
    let retained_updates_after_cached_reads =
        engine.retained_style_system_update_count_for_document_for_test(document);

    let world = engine.world_for_document(document);
    engine.publish_screen_styles(document, world.document_state.screen_style_coverage());
    engine.ensure_retained_style_system_for_document(&host, document, viewport_key, &inputs);

    assert_eq!(
        engine.source_set_generation_for_document_for_test(document),
        source_set_generation_after_cached_reads,
        "a device update must not advance source-set generation"
    );
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        computed_generation_after_cached_reads,
        "an in-place device update must not create a full-cache generation boundary"
    );
    assert!(
        engine.retained_style_system_generation_for_document_for_test(document)
            > retained_generation_after_cached_reads,
        "the observation generation must record the in-place device update"
    );
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        retained_identity_after_cached_reads,
        "viewport changes must retain the Document Stylist"
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        retained_rebuilds_after_cached_reads
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        retained_updates_after_cached_reads + 1
    );
    world
        .document_state
        .note_screen_style_attribute_changes([target]);
    world.document_state.record_style_targets(
        &host,
        &engine.dom_adapter,
        document,
        [(target, StyleInvalidationScope::Element)],
    );
    world.document_state.finish_screen_style_invalidation();
    assert!(
        matches!(
            *world.document_state.screen_style_invalidation.borrow(),
            ScreenStyleInvalidation::Full
        ),
        "attribute dependency coverage expires when the retained world is updated"
    );

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &inputs,
            next_viewport,
        ),
        Some("rgb(4, 5, 6)".to_owned()),
        "the retained world must recascade media-query-dependent values"
    );
}
#[test]
fn stylesheet_source_clones_share_css_text_and_base_url() {
    let base_url = url::Url::parse("https://example.test/assets/app.css").unwrap();
    let source = StyloStylesheetSource::new(".probe { color: green; }".to_owned(), base_url);
    let cloned = source.clone();

    assert!(source.shares_source_storage_for_test(&cloned));
    assert!(StdArc::ptr_eq(&source.base_url, &cloned.base_url));
    assert_eq!(source, cloned);
}
#[test]
fn source_scope_fallback_roots_preserve_unrelated_document_cache() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let active_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &active_url,
                active,
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
                &active_url,
                detached,
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
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );

    assert!(engine.invalidate_style_subtrees(&host, [document]));

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1,
        "document fallback records a lazy root instead of enumerating published descendants"
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert_eq!(
        engine.retained_style_invalidation_root_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.retained_style_invalidation_root_count_for_document_for_test(detached_document),
        0
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
}
#[test]
fn source_scope_fallback_roots_preserve_active_document_cache_for_detached_document() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let active_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &active_url,
                active,
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
                &active_url,
                detached,
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
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );

    assert!(engine.invalidate_style_subtrees(&host, [detached_document]));

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1,
        "detached-document fallback records a lazy root in only that document world"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
    assert_eq!(
        engine.retained_style_invalidation_root_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.retained_style_invalidation_root_count_for_document_for_test(detached_document),
        1
    );
}
#[test]
fn state_change_without_snapshot_invalidates_only_the_owner_document() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("button");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let mut engine = MoliStyleEngine::new();
    let active_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![StyloStylesheetSource::new(
            "button:hover { color: red; }".into(),
            active_url.clone(),
        )],
        ..Default::default()
    };
    for handle in [active, detached] {
        assert!(
            engine
                .computed_style_property_value(
                    &host,
                    &active_url,
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        active,
        StyloElementState::HOVER,
        None,
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1,
        "source-scope fallback must defer descendant recascade until observation"
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert_eq!(
        engine.retained_style_invalidation_root_count_for_document_for_test(document),
        1
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
}
