use super::*;

#[test]
fn attr_attribute_invalidation_preserves_unrelated_cached_styles() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    let child = host.create_element("span");
    let unrelated = host.create_element("div");
    for element in [target, unrelated] {
        assert!(host.set_attribute(element, "class", "target"));
        assert!(host.set_attribute(element, "data-value", "11"));
        assert!(host.append_child(document, element));
    }
    assert!(host.set_attribute(child, "class", "consumer"));
    assert!(host.append_child(target, child));
    let shadow_host = host.create_element("aside");
    assert!(host.append_child(document, shadow_host));
    let shadow_root = host.attach_shadow_root(shadow_host, "open").unwrap();
    let document_url = url::Url::parse("https://attr-invalidation.test/").unwrap();
    let source = StyloStylesheetSource::new(
        ".target { --index: attr(data-value type(<integer>), 7); z-index: var(--index); }
         .consumer { z-index: var(--index); }"
            .into(),
        document_url.clone(),
    );
    let mut engine = MoliStyleEngine::new();
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let shadow_source =
        StyloStylesheetSource::new(":host { color: blue; }".into(), document_url.clone());
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(source);
    inputs
        .shadow_stylesheet_sources
        .push((shadow_root, vec![shadow_source]));
    let read = |engine: &MoliStyleEngine, host: &DomHost, element| {
        engine
            .computed_style_property_value(
                host,
                &document_url,
                element,
                "z-index",
                None,
                &inputs,
                None,
            )
            .expect("computed z-index")
    };
    for element in [target, child, unrelated] {
        assert_eq!(read(&engine, &host, element), "11");
    }
    let target_before = retained_primary_style_for_test(&engine, &host, target).unwrap();
    let child_before = retained_primary_style_for_test(&engine, &host, child).unwrap();
    let unrelated_before = retained_primary_style_for_test(&engine, &host, unrelated).unwrap();
    let generation = engine.style_invalidation_generation_for_document_for_test(document);
    let rebuilds = engine.retained_style_system_rebuild_count_for_document_for_test(document);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();

    let shadow_cascade =
        engine.with_retained_style_system_for_document_for_test(document, |retained| {
            retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == shadow_root)
                .unwrap()
                .1
                .clone()
        });
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(document, shadow_root, shadow_cascade);

    assert!(host.set_attribute(target, "data-unrelated", "ignored"));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: target,
            name: "data-unrelated".into(),
            old_value: None,
            new_value: Some("ignored".into()),
        }],
        &media,
    );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(shadow_root),
        "looking up attribute dependencies must preserve unrelated shadow cascades"
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert_eq!(read(&engine, &host, child), "11");
    assert_eq!(
        engine.style_invalidation_generation_for_document_for_test(document),
        generation,
        "unreferenced attributes should not invalidate computed styles"
    );
    for (element, before) in [(target, &target_before), (child, &child_before)] {
        let after = retained_primary_style_for_test(&engine, &host, element).unwrap();
        assert!(ServoArc::ptr_eq(before, &after));
    }

    assert!(host.set_attribute(target, "data-value", "23"));
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::Attribute {
            element: target,
            name: "data-value".into(),
            old_value: Some("11".into()),
            new_value: Some("23".into()),
        }],
        &media,
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert_eq!(read(&engine, &host, child), "23");
    assert_eq!(read(&engine, &host, target), "23");
    assert_eq!(read(&engine, &host, unrelated), "11");
    let unrelated_after = retained_primary_style_for_test(&engine, &host, unrelated).unwrap();
    assert!(ServoArc::ptr_eq(&unrelated_before, &unrelated_after));
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds,
        "attribute values do not change the retained stylesheet world"
    );
}
