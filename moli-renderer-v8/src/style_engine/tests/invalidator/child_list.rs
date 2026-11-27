use super::*;

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

    assert!(host.remove_child(container, removed));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: container,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed],
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
fn ordinary_sibling_selectors_do_not_invalidate_an_unrelated_relative_anchor() {
    let mut host = test_host();
    let document = host.document_handle();
    let body = host.create_element("body");
    let list = host.create_element("div");
    let first = host.create_element("span");
    let second = host.create_element("span");
    let inserted = host.create_element("span");
    let unrelated = host.create_element("section");
    assert!(host.append_child(document, body));
    assert!(host.append_child(body, list));
    assert!(host.append_child(body, unrelated));
    for item in [first, second, inserted] {
        assert!(host.set_attribute(item, "class", "item"));
    }
    assert!(host.append_child(list, first));
    assert!(host.append_child(list, second));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".item + .item { color: blue; } body:has(.never) { color: red; }".into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    for (element, color) in [(second, "rgb(0, 0, 255)"), (unrelated, "rgb(0, 0, 0)")] {
        assert_eq!(
            engine.computed_style_property_value(
                &host,
                &document_url,
                element,
                "color",
                None,
                &inputs,
                None,
            ),
            Some(color.into()),
        );
    }

    assert!(host.append_child(list, inserted));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: list,
            added_nodes: vec![inserted],
            removed_nodes: Vec::new(),
            previous_sibling: Some(second),
            next_sibling: None,
        }],
        &crate::protocol_types::EmulatedMediaOverrides::default(),
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, unrelated));
}

#[test]
fn reordered_summary_invalidates_ua_styles_when_author_source_has_no_target() {
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
                previous_sibling: Some(first),
                next_sibling: None,
            },
            StyleMutationEffect::ChildList {
                parent: details,
                added_nodes: vec![second],
                removed_nodes: Vec::new(),
                previous_sibling: None,
                next_sibling: Some(first),
            },
        ],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    // Reordering uses lazy source-scope invalidation, not eager eviction of
    // exact sibling targets. Both cached styles must be stale before a read.
    let world = engine.world_for_document(document);
    let invalidation = &world.document_state.style_paths;
    for handle in [first, second] {
        let generation = std::iter::successors(Some(handle), |&node| host.parent_node(node))
            .map(|node| invalidation.subtree_generation(node))
            .max()
            .unwrap();
        assert!(
            engine
                .dom_adapter
                .style_path(document, handle)
                .map_or(0, |path| path.observed_at)
                < generation
                || engine
                    .dom_adapter
                    .clone_clean_element_styles(document, handle, 0)
                    .is_none()
        );
    }
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
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    assert!(host.set_attribute(dialog, "open", ""));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: dialog,
            namespace: None,
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

    assert!(host.remove_child(main, blocker));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent: main,
            added_nodes: Vec::new(),
            removed_nodes: vec![blocker],
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
    assert!(
        !engine.computed_style_cache_contains_handle_for_document_for_test(document, subject),
        "the native relative invalidator should reach the affected subject"
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

    assert!(host.remove_child(parent, removed));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::ChildList {
            parent,
            added_nodes: Vec::new(),
            removed_nodes: vec![removed],
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
