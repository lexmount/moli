use super::*;

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
    host.set_active_element_handle(Some(source));
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
    let old_state = engine.retained_current_element_state(&host, source);
    assert!(host.set_checked_state(source, true));
    engine.invalidate_for_element_state_change_with_old_state(
        &host,
        source,
        StyloElementState::CHECKED,
        old_state,
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
            namespace: None,
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
fn retained_stylo_invalidator_accepts_inactive_media_dependency_as_exact_empty() {
    let mut host = test_host();
    let document = host.document_handle();
    let source = host.create_element("div");
    let target = host.create_element("span");
    let unrelated = host.create_element("aside");

    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "id", "unrelated"));
    assert!(host.append_child(document, source));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let stylesheet = StyloStylesheetSource::new(
        "@media print {
             .probe + .target { color: red; }
         }
         #unrelated { color: rgb(1, 2, 3); }"
            .into(),
        document_url.clone(),
    );
    engine.set_document_adopted_style_sheet_sources(document, vec![stylesheet.clone()]);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(stylesheet);

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
        Some("rgb(0, 0, 0)".into())
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

    assert!(host.set_attribute(source, "class", "probe"));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: source,
            namespace: None,
            name: "class".into(),
            old_value: None,
            new_value: Some("probe".into()),
        }],
        &crate::protocol_types::EmulatedMediaOverrides::default(),
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, target));
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
        Some("rgb(0, 0, 0)".into())
    );
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
    assert!(host.set_document_target_element(document, Some(source)));
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
                namespace: None,
                name: "class".to_owned(),
                old_value: Some("marker".to_owned()),
                new_value: Some("other".to_owned()),
            },
            StyleMutationEffect::Attribute {
                element: attr_loose_marker,
                namespace: None,
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
                namespace: None,
                name: "class".to_owned(),
                old_value: Some("marker".to_owned()),
                new_value: Some("other".to_owned()),
            },
            StyleMutationEffect::Attribute {
                element: attr_loose_marker,
                namespace: None,
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
