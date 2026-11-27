use super::*;

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
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);
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
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);
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
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);
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
            namespace: None,
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
            namespace: None,
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
