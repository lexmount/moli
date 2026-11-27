use super::*;

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
