use super::*;

fn set_class(engine: &mut MoliStyleEngine, host: &mut DomHost, element: DomHandle, value: &str) {
    let old_value = host
        .element(element)
        .unwrap()
        .attribute("class")
        .map(str::to_owned);
    assert!(host.set_attribute(element, "class", value));
    engine.invalidate_for_mutations(
        host,
        &[StyleMutationEffect::Attribute {
            element,
            namespace: None,
            name: "class".to_owned(),
            old_value,
            new_value: Some(value.to_owned()),
        }],
        &crate::protocol_types::EmulatedMediaOverrides::default(),
    );
}

#[test]
fn native_snapshot_can_invalidate_a_sibling_of_an_unstyled_source() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let trigger = host.create_element("div");
    let target = host.create_element("span");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, trigger));
    assert!(host.append_child(parent, target));

    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            "span { color: blue } @scope (section) { .active + span { color: red } }".into(),
            url.clone(),
        ));
    let color = |engine: &MoliStyleEngine, host: &DomHost| {
        engine
            .computed_style_property_value(host, &url, target, "color", None, &inputs, None)
            .unwrap()
    };
    assert_eq!(color(&engine, &host), "rgb(0, 0, 255)");
    assert!(!engine.dom_adapter.has_element_data(trigger));

    // The drain must use the first old attribute and the latest live value.
    set_class(&mut engine, &mut host, trigger, "intermediate");
    set_class(&mut engine, &mut host, trigger, "active");
    assert!(
        !engine
            .world_for_document(document)
            .pending_invalidations
            .borrow()
            .snapshots
            .is_empty()
    );
    assert_eq!(color(&engine, &host), "rgb(255, 0, 0)");
    assert!(!engine.dom_adapter.has_element_data(trigger));

    set_class(&mut engine, &mut host, trigger, "intermediate");
    set_class(&mut engine, &mut host, trigger, "inactive");
    assert_eq!(color(&engine, &host), "rgb(0, 0, 255)");

    set_class(&mut engine, &mut host, trigger, "active");
    assert_eq!(color(&engine, &host), "rgb(255, 0, 0)");
    set_class(&mut engine, &mut host, trigger, "inactive");
    let removal = host.remove_child_effects(parent, trigger);
    engine.invalidate_for_mutations(
        &host,
        &StyleMutationEffect::from_dom_mutation_effects(&removal),
        &crate::protocol_types::EmulatedMediaOverrides::default(),
    );
    assert_eq!(color(&engine, &host), "rgb(0, 0, 255)");
}

#[test]
fn native_snapshot_pruning_preserves_sparse_reads_inside_hidden_subtrees() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let hidden = host.create_element("div");
    let target = host.create_element("span");
    let untouched = host.create_element("span");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, hidden));
    assert!(host.append_child(hidden, target));
    assert!(host.append_child(hidden, untouched));
    for _ in 0..10_000 {
        let child = host.create_element("span");
        assert!(host.append_child(hidden, child));
    }

    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            "section > div { display: none } span { color: blue } .active span { color: red }"
                .into(),
            url.clone(),
        ));
    let color = |engine: &MoliStyleEngine, host: &DomHost| {
        engine
            .computed_style_property_value(host, &url, target, "color", None, &inputs, None)
            .unwrap()
    };
    assert_eq!(color(&engine, &host), "rgb(0, 0, 255)");
    let cache_entries = engine.computed_style_cache_entry_count_for_document_for_test(document);

    set_class(&mut engine, &mut host, parent, "active");
    assert!(
        !engine
            .world_for_document(document)
            .pending_invalidations
            .borrow()
            .snapshots
            .is_empty()
    );
    assert_eq!(color(&engine, &host), "rgb(255, 0, 0)");
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        cache_entries
    );
    assert!(!engine.dom_adapter.has_element_data(untouched));
}

#[test]
fn native_snapshot_batch_observes_attribute_and_state_changes_together() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let trigger = host.create_element("input");
    let target = host.create_element("span");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, trigger));
    assert!(host.append_child(parent, target));
    assert!(host.set_attribute(trigger, "type", "checkbox"));
    assert!(host.set_attribute(trigger, "class", "active"));
    assert!(host.set_checked_state(trigger, true));

    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            "span { color: blue } .active:checked + span { color: red }".into(),
            url.clone(),
        ));
    let color = |engine: &MoliStyleEngine, host: &DomHost| {
        engine
            .computed_style_property_value(host, &url, target, "color", None, &inputs, None)
            .unwrap()
    };
    assert_eq!(color(&engine, &host), "rgb(255, 0, 0)");
    let old_state = engine
        .retained_current_element_state(&host, trigger)
        .unwrap();

    set_class(&mut engine, &mut host, trigger, "inactive");
    assert!(host.set_checked_state(trigger, false));
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        trigger,
        dom::ElementState::CHECKED,
        Some(old_state),
        &crate::protocol_types::EmulatedMediaOverrides::default(),
    );
    assert_eq!(color(&engine, &host), "rgb(0, 0, 255)");
}

#[test]
fn media_playing_invalidates_cached_style_without_a_paused_rule() {
    let mut host = test_host();
    let document = host.document_handle();
    let video = host.create_element("video");
    assert!(host.append_child(document, video));

    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "video { color: blue } video:playing { color: red }".into(),
        url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    let color = |engine: &MoliStyleEngine, host: &DomHost| {
        engine
            .computed_style_property_value(host, &url, video, "color", None, &inputs, None)
            .unwrap()
    };
    assert_eq!(color(&engine, &host), "rgb(0, 0, 255)");

    let emulated_media = crate::protocol_types::EmulatedMediaOverrides::default();
    for (paused, expected) in [(false, "rgb(255, 0, 0)"), (true, "rgb(0, 0, 255)")] {
        let old_state = engine.retained_current_element_state(&host, video).unwrap();
        assert!(host.set_media_paused(video, paused));
        engine.invalidate_for_element_state_change_with_old_state(
            &host,
            video,
            dom::ElementState::PAUSED,
            Some(old_state),
            &emulated_media,
        );
        assert_eq!(color(&engine, &host), expected);
    }
}

#[test]
fn namespaced_attribute_mutation_updates_a_cached_sibling() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let trigger = host.create_element("div");
    let target = host.create_element("span");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, trigger));
    assert!(host.append_child(parent, target));

    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        r#"@namespace t "urn:test"; span { color: blue } [t|data-mode="on"] + span { color: red }"#
            .into(),
        url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    let color = |engine: &MoliStyleEngine, host: &DomHost| {
        engine
            .computed_style_property_value(host, &url, target, "color", None, &inputs, None)
            .unwrap()
    };
    assert_eq!(color(&engine, &host), "rgb(0, 0, 255)");

    let set = host.set_attribute_ns_effects(
        trigger,
        Some("urn:test"),
        None,
        "data-mode",
        "data-mode",
        "on",
    );
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &StyleMutationEffect::from_dom_mutation_effects(&set),
        &media,
    );
    assert!(
        !engine
            .world_for_document(document)
            .pending_invalidations
            .borrow()
            .snapshots
            .is_empty()
    );
    assert_eq!(color(&engine, &host), "rgb(255, 0, 0)");

    let remove = host.remove_attribute_ns_effects(trigger, Some("urn:test"), "data-mode");
    engine.invalidate_for_mutations(
        &host,
        &StyleMutationEffect::from_dom_mutation_effects(&remove),
        &media,
    );
    assert_eq!(color(&engine, &host), "rgb(0, 0, 255)");
}

#[test]
fn mixed_native_snapshot_and_splice_preserve_unrelated_styles() {
    let mut host = test_host();
    let document = host.document_handle();
    let root = host.create_element("main");
    let parent = host.create_element("section");
    let target = host.create_element("span");
    let list = host.create_element("ul");
    let unrelated = host.create_element("span");
    assert!(host.append_child(document, root));
    assert!(host.append_child(root, parent));
    assert!(host.append_child(parent, target));
    assert!(host.append_child(root, list));
    assert!(host.append_child(root, unrelated));

    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            "span { color: blue } .active { color: red }".into(),
            url.clone(),
        ));
    let color = |engine: &MoliStyleEngine, host: &DomHost, node| {
        engine
            .computed_style_property_value(host, &url, node, "color", None, &inputs, None)
            .unwrap()
    };
    assert_eq!(color(&engine, &host, target), "rgb(0, 0, 255)");
    assert_eq!(color(&engine, &host, unrelated), "rgb(0, 0, 255)");

    set_class(&mut engine, &mut host, target, "active");
    assert!(
        !engine
            .world_for_document(document)
            .pending_invalidations
            .borrow()
            .snapshots
            .is_empty()
    );
    let item = host.create_element("li");
    let append = host.append_child_effects(list, item);
    engine.invalidate_for_mutations(
        &host,
        &StyleMutationEffect::from_dom_mutation_effects(&append),
        &crate::protocol_types::EmulatedMediaOverrides::default(),
    );
    assert_eq!(color(&engine, &host, target), "rgb(255, 0, 0)");
    let resolutions = engine.element_style_resolution_count_for_document_for_test(document);
    assert_eq!(color(&engine, &host, unrelated), "rgb(0, 0, 255)");
    assert_eq!(
        engine.element_style_resolution_count_for_document_for_test(document),
        resolutions
    );
}

#[test]
fn removal_with_a_pending_attribute_snapshot_refreshes_relative_anchor() {
    let mut host = test_host();
    let document = host.document_handle();
    let anchor = host.create_element("section");
    let branch = host.create_element("div");
    let marker = host.create_element("span");
    let outside = host.create_element("aside");
    assert!(host.set_attribute(marker, "class", "marker"));
    assert!(host.append_child(document, anchor));
    assert!(host.append_child(anchor, branch));
    assert!(host.append_child(branch, marker));
    assert!(host.append_child(document, outside));

    let mut engine = MoliStyleEngine::new();
    let url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![StyloStylesheetSource::new(
            "section { color: blue } section:has(.marker) { color: red }".into(),
            url.clone(),
        )],
        ..Default::default()
    };
    let color = |engine: &MoliStyleEngine, host: &DomHost, element| {
        engine
            .computed_style_property_value(host, &url, element, "color", None, &inputs, None)
            .unwrap()
    };
    assert_eq!(color(&engine, &host, anchor), "rgb(255, 0, 0)");
    let _ = color(&engine, &host, outside);

    set_class(&mut engine, &mut host, marker, "inactive");
    let removal = host.remove_child_effects(anchor, branch);
    engine.invalidate_for_mutations(
        &host,
        &StyleMutationEffect::from_dom_mutation_effects(&removal),
        &crate::protocol_types::EmulatedMediaOverrides::default(),
    );

    assert_eq!(color(&engine, &host, anchor), "rgb(0, 0, 255)");
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, outside));
}

#[test]
fn coalesced_relative_removal_preserves_siblings_unless_a_selector_reaches_them() {
    for suffix in [" > p", " + aside"] {
        let mut host = test_host();
        let document = host.document_handle();
        let root = host.create_element("main");
        let anchor = host.create_element("section");
        let branch = host.create_element("div");
        let marker = host.create_element("span");
        let child = host.create_element("p");
        let sibling = host.create_element("aside");
        assert!(host.set_attribute(marker, "class", "marker"));
        assert!(host.append_child(document, root));
        assert!(host.append_child(root, anchor));
        assert!(host.append_child(anchor, branch));
        assert!(host.append_child(branch, marker));
        assert!(host.append_child(anchor, child));
        assert!(host.append_child(root, sibling));

        let mut engine = MoliStyleEngine::new();
        let url = url::Url::parse("https://example.test/").unwrap();
        let inputs = FullStyleWorldSnapshot {
            document_stylesheet_sources: vec![StyloStylesheetSource::new(
                format!("p, aside {{ color: blue }} section:has(.marker){suffix} {{ color: red }}"),
                url.clone(),
            )],
            ..Default::default()
        };
        let color = |engine: &MoliStyleEngine, host: &DomHost, element| {
            engine
                .computed_style_property_value(host, &url, element, "color", None, &inputs, None)
                .unwrap()
        };
        let dependent = if suffix == " > p" { child } else { sibling };
        assert_eq!(color(&engine, &host, dependent), "rgb(255, 0, 0)");
        let _ = color(&engine, &host, sibling);

        set_class(&mut engine, &mut host, marker, "inactive");
        let removal = host.remove_child_effects(anchor, branch);
        engine.invalidate_for_mutations(
            &host,
            &StyleMutationEffect::from_dom_mutation_effects(&removal),
            &crate::protocol_types::EmulatedMediaOverrides::default(),
        );

        assert_eq!(
            color(&engine, &host, dependent),
            "rgb(0, 0, 255)",
            "{suffix}"
        );
        if dependent != sibling {
            assert!(
                engine
                    .computed_style_cache_contains_handle_for_document_for_test(document, sibling)
            );
        }
    }
}
