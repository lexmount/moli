use super::*;

#[test]
fn source_cascade_data_materializes_only_for_targeted_invalidations() {
    reset_source_cascade_rebuild_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let async_style = host.create_element("style");
    let state_style = host.create_element("style");
    let script = host.create_element("script");
    let state_target = host.create_element("div");
    for child in [async_style, state_style, script, state_target] {
        assert!(host.append_child(document, child));
    }

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        async_style,
        "script[async] { color: red; }".into(),
    );
    engine.set_owner_style_sheet_text_with_host(
        &host,
        state_style,
        "div[data-state] { color: blue; }".into(),
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);

    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for target in [script, state_target] {
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
    }
    let async_source_id =
        StyleSourceId::owner_style_sheet(&host, async_style).expect("async style source id");
    let state_source_id =
        StyleSourceId::owner_style_sheet(&host, state_style).expect("state style source id");
    engine.with_retained_style_system_for_document_for_test(document, |retained| {
        assert!(retained.source_cascade_projections.is_empty());
    });
    assert_eq!(source_cascade_rebuild_count_for_test(), 0);
    let retained_updates =
        engine.retained_style_system_update_count_for_document_for_test(document);

    assert!(host.set_attribute(script, "async", ""));
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: script,
            name: "async".into(),
            old_value: None,
            new_value: Some(String::new()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    engine.with_retained_style_system_for_document_for_test(document, |retained| {
        assert!(
            retained
                .source_cascade_projections
                .contains_key(&async_source_id)
        );
        assert!(
            !retained
                .source_cascade_projections
                .contains_key(&state_source_id)
        );
    });
    assert_eq!(source_cascade_rebuild_count_for_test(), 1);
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        retained_updates,
        "derived cache materialization must not count as a style-world update"
    );

    assert!(host.set_attribute(state_target, "data-state", "active"));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: state_target,
            name: "data-state".into(),
            old_value: None,
            new_value: Some("active".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    engine.with_retained_style_system_for_document_for_test(document, |retained| {
        assert!(
            retained
                .source_cascade_projections
                .contains_key(&async_source_id)
        );
        assert!(
            retained
                .source_cascade_projections
                .contains_key(&state_source_id)
        );
    });
    assert_eq!(source_cascade_rebuild_count_for_test(), 2);

    assert!(host.set_attribute(script, "async", "true"));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: script,
            name: "async".into(),
            old_value: Some(String::new()),
            new_value: Some("true".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert_eq!(
        source_cascade_rebuild_count_for_test(),
        2,
        "warm invalidations should reuse a materialized source cascade"
    );
}
