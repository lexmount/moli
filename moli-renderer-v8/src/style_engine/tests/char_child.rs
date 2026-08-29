use super::*;

#[test]
fn character_data_without_structural_selector_preserves_cache_entries() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("p");
    let text = host.create_text_node("before");
    let target = host.create_element("span");

    assert!(host.set_attribute(parent, "class", "parent"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, text));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".parent + .target { color: red; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [parent, target] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::CharacterData { node: text }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, parent));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}
#[test]
fn character_data_structural_sibling_dependency_invalidates_following_siblings() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("p");
    let text = host.create_text_node("");
    let target = host.create_element("span");

    assert!(host.set_attribute(parent, "class", "parent"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, text));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".parent:empty + .target { color: red; }".into(),
            document_url.clone(),
        )],
    );
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [parent, target] {
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::CharacterData { node: text }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}
#[test]
fn character_data_has_empty_dependency_invalidates_ancestor_subjects() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let child = host.create_element("p");
    let text = host.create_text_node("");
    let target = host.create_element("span");

    assert!(host.set_attribute(container, "class", "container"));
    assert!(host.set_attribute(child, "class", "child"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, child));
    assert!(host.append_child(child, text));
    assert!(host.append_child(container, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".container:has(.child:empty) .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [child, target] {
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
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::CharacterData { node: text }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0,
        "the exact :has(:empty) target query invalidates both published targets"
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
}

#[test]
fn child_list_has_empty_dependency_invalidates_ancestor_subjects() {
    let mut host = test_host();
    let document = host.document_handle();
    let subject = host.create_element("section");
    let child = host.create_element("div");

    assert!(host.set_attribute(subject, "id", "subject"));
    assert!(host.set_attribute(child, "id", "child"));
    assert!(host.append_child(document, subject));
    assert!(host.append_child(subject, child));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "#subject { display: block; } #subject:has(:empty) { display: none; }".into(),
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
            "display",
            None,
            &inputs,
            None,
        ),
        Some("none".into())
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    let text = host.create_text_node("text");
    assert!(host.append_child(child, text));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: child,
            added_nodes: vec![text],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: None,
            next_sibling: None,
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            subject,
            "display",
            None,
            &inputs,
            None,
        ),
        Some("block".into())
    );
}
#[test]
fn child_list_type_sibling_invalidation_clears_next_sibling_without_keyed_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let inserted = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source =
        StyloStylesheetSource::new("div + .target { color: red; }".into(), document_url.clone());
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
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));

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
}
#[test]
fn child_list_universal_sibling_invalidation_keeps_later_sibling_cache() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let inserted = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source =
        StyloStylesheetSource::new("* + .target { color: red; }".into(), document_url.clone());
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
}
#[test]
fn child_list_universal_sibling_removal_keeps_later_sibling_cache() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let removed = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, removed));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source =
        StyloStylesheetSource::new("* + .target { color: red; }".into(), document_url.clone());
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

    let removed_snapshot = removed_element_dependency_snapshots(&host, &[removed]);
    assert!(host.remove_child(document, removed));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: document,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed],
            removed_element_snapshots: removed_snapshot,
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
fn retained_stylo_invalidator_uses_batch_snapshot_for_outer_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let scoped_outer = host.create_element("div");
    let scoped = host.create_element("section");
    let scoped_marker = host.create_element("div");
    let scoped_middle = host.create_element("div");
    let scoped_target = host.create_element("p");
    let loose_outer = host.create_element("div");
    let loose = host.create_element("section");
    let loose_marker = host.create_element("div");
    let loose_middle = host.create_element("div");
    let loose_target = host.create_element("p");
    assert!(host.set_attribute(scoped_middle, "class", "scope"));
    assert!(host.set_attribute(scoped_marker, "class", "marker"));
    assert!(host.set_attribute(scoped_target, "class", "target"));
    assert!(host.set_attribute(loose_marker, "class", "marker"));
    assert!(host.set_attribute(loose_target, "class", "target"));
    assert!(host.append_child(document, scoped_outer));
    assert!(host.append_child(scoped_outer, scoped));
    assert!(host.append_child(scoped, scoped_marker));
    assert!(host.append_child(scoped_marker, scoped_middle));
    assert!(host.append_child(scoped_middle, scoped_target));
    assert!(host.append_child(document, loose_outer));
    assert!(host.append_child(loose_outer, loose));
    assert!(host.append_child(loose, loose_marker));
    assert!(host.append_child(loose_marker, loose_middle));
    assert!(host.append_child(loose_middle, loose_target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "div .scope:where(.marker *) .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [scoped_target, loose_target] {
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

    assert!(host.set_attribute(scoped_marker, "class", "other"));
    assert!(host.set_attribute(loose_marker, "class", "other"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let effects = [
        StyleMutationEffect::Attribute {
            element: scoped_marker,
            name: "class".to_owned(),
            old_value: Some("marker".to_owned()),
            new_value: Some("other".to_owned()),
        },
        StyleMutationEffect::Attribute {
            element: loose_marker,
            name: "class".to_owned(),
            old_value: Some("marker".to_owned()),
            new_value: Some("other".to_owned()),
        },
    ];
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, scoped_target)
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, loose_target)
    );
}
#[test]
fn retained_stylo_invalidator_uses_attribute_value_snapshot_for_outer_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let scoped_outer = host.create_element("div");
    let scoped = host.create_element("section");
    let scoped_marker = host.create_element("div");
    let scoped_middle = host.create_element("div");
    let scoped_target = host.create_element("p");
    let loose_outer = host.create_element("div");
    let loose = host.create_element("section");
    let loose_marker = host.create_element("div");
    let loose_middle = host.create_element("div");
    let loose_target = host.create_element("p");
    assert!(host.set_attribute(scoped_middle, "class", "scope"));
    assert!(host.set_attribute(scoped_marker, "data-state", "old"));
    assert!(host.set_attribute(scoped_target, "class", "target"));
    assert!(host.set_attribute(loose_marker, "data-state", "old"));
    assert!(host.set_attribute(loose_target, "class", "target"));
    assert!(host.append_child(document, scoped_outer));
    assert!(host.append_child(scoped_outer, scoped));
    assert!(host.append_child(scoped, scoped_marker));
    assert!(host.append_child(scoped_marker, scoped_middle));
    assert!(host.append_child(scoped_middle, scoped_target));
    assert!(host.append_child(document, loose_outer));
    assert!(host.append_child(loose_outer, loose));
    assert!(host.append_child(loose, loose_marker));
    assert!(host.append_child(loose_marker, loose_middle));
    assert!(host.append_child(loose_middle, loose_target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "div .scope:where([data-state=\"old\"] *) .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [scoped_target, loose_target] {
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

    assert!(host.set_attribute(scoped_marker, "data-state", "new"));
    assert!(host.set_attribute(loose_marker, "data-state", "new"));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let effects = [
        StyleMutationEffect::Attribute {
            element: scoped_marker,
            name: "data-state".to_owned(),
            old_value: Some("old".to_owned()),
            new_value: Some("new".to_owned()),
        },
        StyleMutationEffect::Attribute {
            element: loose_marker,
            name: "data-state".to_owned(),
            old_value: Some("old".to_owned()),
            new_value: Some("new".to_owned()),
        },
    ];
    engine.invalidate_for_mutations(&host, &effects, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, scoped_target)
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, loose_target)
    );
}
#[test]
fn retained_stylo_invalidator_uses_focus_state_snapshot_for_outer_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let scoped_outer = host.create_element("div");
    let scoped = host.create_element("section");
    let scoped_marker = host.create_element("button");
    let scoped_middle = host.create_element("div");
    let scoped_target = host.create_element("p");
    let loose_outer = host.create_element("div");
    let loose = host.create_element("section");
    let loose_marker = host.create_element("button");
    let loose_middle = host.create_element("div");
    let loose_target = host.create_element("p");
    assert!(host.set_attribute(scoped_middle, "class", "scope"));
    assert!(host.set_attribute(scoped_target, "class", "target"));
    assert!(host.set_attribute(loose_target, "class", "target"));
    assert!(host.append_child(document, scoped_outer));
    assert!(host.append_child(scoped_outer, scoped));
    assert!(host.append_child(scoped, scoped_marker));
    assert!(host.append_child(scoped_marker, scoped_middle));
    assert!(host.append_child(scoped_middle, scoped_target));
    assert!(host.append_child(document, loose_outer));
    assert!(host.append_child(loose_outer, loose));
    assert!(host.append_child(loose, loose_marker));
    assert!(host.append_child(loose_marker, loose_middle));
    assert!(host.append_child(loose_middle, loose_target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        "div .scope:where(:focus *) .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [scoped_target, loose_target] {
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

    host.set_active_element_handle(Some(loose_marker));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_focus_change(&host, Some(scoped_marker), Some(loose_marker), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, scoped_target)
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, loose_target)
    );
}
#[test]
fn retained_stylo_invalidator_uses_target_state_snapshot_for_outer_dependency() {
    let mut host = test_host();
    let document = host.document_handle();
    let scoped_outer = host.create_element("div");
    let scoped = host.create_element("section");
    let scoped_marker = host.create_element("a");
    let scoped_middle = host.create_element("div");
    let scoped_target = host.create_element("p");
    let loose_outer = host.create_element("div");
    let loose = host.create_element("section");
    let loose_marker = host.create_element("a");
    let loose_middle = host.create_element("div");
    let loose_target = host.create_element("p");
    assert!(host.set_attribute(scoped_marker, "id", "old"));
    assert!(host.set_attribute(loose_marker, "id", "new"));
    assert!(host.set_attribute(scoped_middle, "class", "scope"));
    assert!(host.set_attribute(scoped_target, "class", "target"));
    assert!(host.set_attribute(loose_target, "class", "target"));
    assert!(host.append_child(document, scoped_outer));
    assert!(host.append_child(scoped_outer, scoped));
    assert!(host.append_child(scoped, scoped_marker));
    assert!(host.append_child(scoped_marker, scoped_middle));
    assert!(host.append_child(scoped_middle, scoped_target));
    assert!(host.append_child(document, loose_outer));
    assert!(host.append_child(loose_outer, loose));
    assert!(host.append_child(loose, loose_marker));
    assert!(host.append_child(loose_marker, loose_middle));
    assert!(host.append_child(loose_middle, loose_target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/#old").unwrap();
    let source = StyloStylesheetSource::new(
        "div .scope:where(:target *) .target { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [scoped_target, loose_target] {
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

    assert!(host.set_document_target_element(document, Some(loose_marker)));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_target_change(&host, Some(scoped_marker), Some(loose_marker), &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, scoped_target)
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, loose_target)
    );
}
#[test]
fn retained_stylo_child_list_insertion_preserves_broken_next_sibling_target() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let inserted = host.create_element("div");
    let target = host.create_element("span");
    let later = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(previous, "class", "previous"));
    assert!(host.set_attribute(inserted, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(later, "class", "later"));
    assert!(host.set_attribute(unrelated, "class", "unrelated"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, later));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".previous + .target { color: red; } .marker ~ .later { color: green; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for handle in [target, later, unrelated] {
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
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, later));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn retained_stylo_child_list_insertion_uses_element_sibling_relation_across_text() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let previous = host.create_element("p");
    let inserted = host.create_element("div");
    let text = host.create_text_node(" ");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(inserted, "class", "marker"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, previous));
    assert!(host.append_child(container, text));
    assert!(host.append_child(container, target));
    assert!(host.append_child(container, unrelated));

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

    assert!(host.insert_before(container, inserted, Some(text)));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: container,
            added_nodes: vec![inserted],
            removed_nodes: Vec::new(),
            removed_element_snapshots: Vec::new(),
            previous_sibling: Some(previous),
            next_sibling: Some(text),
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
fn retained_stylo_child_list_removal_uses_mutation_time_removed_snapshot() {
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
        ".marker + .target { color: red; } .other + .unrelated { color: blue; }".into(),
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
    let effects = [StyleMutationEffect::ChildList {
        parent: document,
        added_nodes: Vec::new(),
        removed_nodes: vec![removed],
        removed_element_snapshots: removed_element_dependency_snapshots(&host, &[removed]),
        previous_sibling: Some(previous),
        next_sibling: Some(target),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);
    assert!(host.set_attribute(removed, "class", "other"));
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert!(!engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}
#[test]
fn retained_stylo_child_list_removal_preserves_new_previous_sibling_match() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let removed = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(previous, "class", "marker"));
    assert!(host.set_attribute(removed, "class", "spacer"));
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
fn retained_stylo_child_list_removal_uses_element_sibling_relation_across_text() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let previous = host.create_element("p");
    let removed = host.create_element("div");
    let text = host.create_text_node(" ");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(previous, "class", "marker"));
    assert!(host.set_attribute(removed, "class", "spacer"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, previous));
    assert!(host.append_child(container, removed));
    assert!(host.append_child(container, text));
    assert!(host.append_child(container, target));
    assert!(host.append_child(container, unrelated));

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

    assert!(host.remove_child(container, removed));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: container,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed],
            removed_element_snapshots: removed_element_dependency_snapshots(&host, &[removed]),
            previous_sibling: Some(previous),
            next_sibling: Some(text),
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
fn retained_stylo_child_list_removal_uses_previous_element_sibling_across_text() {
    let mut host = test_host();
    let document = host.document_handle();
    let container = host.create_element("section");
    let previous = host.create_element("p");
    let text = host.create_text_node(" ");
    let removed = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(previous, "class", "marker"));
    assert!(host.set_attribute(removed, "class", "spacer"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, container));
    assert!(host.append_child(container, previous));
    assert!(host.append_child(container, text));
    assert!(host.append_child(container, removed));
    assert!(host.append_child(container, target));
    assert!(host.append_child(container, unrelated));

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

    assert!(host.remove_child(container, removed));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: container,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed],
            removed_element_snapshots: removed_element_dependency_snapshots(&host, &[removed]),
            previous_sibling: Some(text),
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
fn retained_stylo_child_list_multi_removal_uses_removed_element_sibling_relation() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let removed_marker = host.create_element("div");
    let removed_spacer = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(removed_marker, "class", "marker"));
    assert!(host.set_attribute(removed_spacer, "class", "spacer"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, removed_marker));
    assert!(host.append_child(document, removed_spacer));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".spacer + .target { color: red; }".into(),
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

    assert!(host.remove_child(document, removed_marker));
    assert!(host.remove_child(document, removed_spacer));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: document,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed_marker, removed_spacer],
            removed_element_snapshots: removed_element_dependency_snapshots(
                &host,
                &[removed_marker, removed_spacer],
            ),
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
fn retained_stylo_child_list_multi_removal_preserves_new_previous_sibling_match() {
    let mut host = test_host();
    let document = host.document_handle();
    let previous = host.create_element("p");
    let removed_one = host.create_element("div");
    let removed_two = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("span");

    assert!(host.set_attribute(previous, "class", "marker"));
    assert!(host.set_attribute(removed_one, "class", "spacer"));
    assert!(host.set_attribute(removed_two, "class", "spacer"));
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "target"));
    assert!(host.append_child(document, previous));
    assert!(host.append_child(document, removed_one));
    assert!(host.append_child(document, removed_two));
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

    assert!(host.remove_child(document, removed_one));
    assert!(host.remove_child(document, removed_two));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: document,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed_one, removed_two],
            removed_element_snapshots: removed_element_dependency_snapshots(
                &host,
                &[removed_one, removed_two],
            ),
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
