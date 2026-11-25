use super::*;

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
