use super::*;

#[test]
fn document_stylesheet_fallback_updates_the_persistent_world_in_place() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, target));
    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".target { color: rgb(1, 2, 3); }".into(),
            document_url.clone(),
        ));
    let first_key = StyleWorldKey::new_for_observation(
        &first_inputs,
        StyleViewport::default(),
        StyleTreeScopeVersions::current(&host, Some(document)),
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
    assert!(engine.retained_style_system_matches_for_document_for_test(document, &first_key));
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);
    let rebuilds = engine.retained_style_system_rebuild_count_for_document_for_test(document);
    let updates = engine.retained_style_system_update_count_for_document_for_test(document);
    let cache_entries = engine.computed_style_cache_entry_count_for_document_for_test(document);

    engine.mark_document_stylesheet_set_dirty(document);

    assert!(engine.retained_style_system_matches_for_document_for_test(document, &first_key));
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "marking a source set dirty must not eagerly replace the Stylist"
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        cache_entries,
        "the last published style remains readable until the next observation"
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::DocumentStyleSheets]
    );

    let mut second_inputs = FullStyleWorldSnapshot::default();
    second_inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".target { color: rgb(4, 5, 6); }".into(),
            document_url.clone(),
        ));
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
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "a full-document source fallback must still update the Stylist in place"
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        updates + 1
    );
    assert!(
        engine
            .source_dirty_scope_reasons_for_document_for_test(document)
            .is_empty()
    );
}

#[test]
fn style_subtree_invalidation_retains_style_system() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("section");
    assert!(host.append_child(document, target));
    let mut engine = MoliStyleEngine::new();
    let inputs = FullStyleWorldSnapshot::default();
    let key = StyleWorldKey::new(&inputs, None);

    engine.ensure_retained_style_system_for_document(
        &host,
        host.document_handle(),
        key.clone(),
        &inputs,
    );
    engine.invalidate_style_subtree(&host, target);

    assert!(engine.retained_style_system_matches_for_document_for_test(document, &key));
    assert!(engine.computed_style_cache_entry_count_for_document_for_test(document) == 0);
}

#[test]
fn retained_style_system_keeps_cascade_data_for_empty_shadow_scopes() {
    let mut host = test_host();
    let document = host.document_handle();
    let open_host = host.create_element("section");
    let closed_host = host.create_element("article");
    assert!(host.append_child(document, open_host));
    assert!(host.append_child(document, closed_host));
    let open_root = host
        .attach_shadow_root(open_host, "open")
        .expect("open host should accept a shadow root");
    let closed_root = host
        .attach_shadow_root(closed_host, "closed")
        .expect("closed host should accept a shadow root");

    let engine = MoliStyleEngine::new();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((open_root, Vec::new()));
    inputs
        .shadow_stylesheet_sources
        .push((closed_root, Vec::new()));
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);

    engine.with_retained_style_system_for_document_for_test(document, |retained| {
        let roots = retained
            .shadow_cascade_data
            .iter()
            .map(|(root, _)| *root)
            .collect::<Vec<_>>();
        assert_eq!(roots, vec![open_root, closed_root]);
    });
}

#[test]
fn document_stylesheet_change_updates_the_retained_system_in_place() {
    reset_author_source_text_parse_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    let inherited_child = host.create_element("span");
    let unrelated = host.create_element("p");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "unrelated"));
    assert!(host.append_child(target, inherited_child));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/incremental.html").unwrap();
    let source_id = StyleSourceId::document_adopted_style_sheet(document, 41);
    let unrelated_source_id = StyleSourceId::document_adopted_style_sheet(document, 42);
    let unrelated_source = StyloStylesheetSource::new(
        ".unrelated { background-color: rgb(7, 8, 9); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(unrelated_source_id));
    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ".target { color: rgb(1, 2, 3); }".into(),
            document_url.clone(),
        )
        .with_source_id(Some(source_id.clone())),
    );
    first_inputs
        .document_stylesheet_sources
        .push(unrelated_source.clone());
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
        engine.computed_style_property_value(
            &host,
            &document_url,
            unrelated,
            "background-color",
            None,
            &first_inputs,
            None,
        ),
        Some("rgb(7, 8, 9)".into())
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            inherited_child,
            "color",
            None,
            &first_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    let target_style_before = retained_primary_style_for_test(&engine, &host, target)
        .expect("target style should be retained");
    let unrelated_style_before = retained_primary_style_for_test(&engine, &host, unrelated)
        .expect("unrelated style should be retained");
    let inherited_style_before = retained_primary_style_for_test(&engine, &host, inherited_child)
        .expect("inherited child style should be retained");
    assert_eq!(author_source_text_parse_count_for_test(), 2);
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);
    let stylist_flushes = engine.retained_stylist_flush_count_for_document_for_test(document);
    let element_resolutions = engine.element_style_resolution_count_for_document_for_test(document);
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        0
    );

    let mut second_inputs = FullStyleWorldSnapshot::default();
    second_inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ".target { color: rgb(4, 5, 6); }".into(),
            document_url.clone(),
        )
        .with_source_id(Some(source_id)),
    );
    second_inputs
        .document_stylesheet_sources
        .push(unrelated_source);
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
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    let target_style_after = retained_primary_style_for_test(&engine, &host, target)
        .expect("target style should be recomputed");
    let unrelated_style_after = retained_primary_style_for_test(&engine, &host, unrelated)
        .expect("unrelated style should remain retained");
    assert!(!ServoArc::ptr_eq(&target_style_before, &target_style_after));
    assert!(
        ServoArc::ptr_eq(&unrelated_style_before, &unrelated_style_after),
        "Stylo stylesheet invalidation must preserve an unrelated source's canonical style"
    );
    assert!(
        !element_style_is_dirty_for_test(&engine, &host, inherited_child),
        "the inherited child remains published; its dirty-root generation is consumed on demand"
    );
    assert!(
        engine
            .computed_style_cache_contains_handle_for_document_for_test(document, inherited_child),
        "a target read must not enumerate and evict its unobserved descendants"
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            inherited_child,
            "color",
            None,
            &second_inputs,
            None,
        ),
        Some("rgb(4, 5, 6)".into()),
        "a matched ancestor invalidation must propagate to demanded inherited descendants"
    );
    let inherited_style_after = retained_primary_style_for_test(&engine, &host, inherited_child)
        .expect("inherited child should be recomputed on demand");
    assert!(!ServoArc::ptr_eq(
        &inherited_style_before,
        &inherited_style_after
    ));
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "a stylesheet revision must preserve the exact Stylist identity"
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        stylist_flushes + 1,
        "one stylesheet revision must flush the document Stylist once"
    );
    assert!(
        engine.element_style_resolution_count_for_document_for_test(document) > element_resolutions,
        "reading the invalidated target must perform a new element style resolution"
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        1,
        "a stylesheet revision must not replace the document Stylist"
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        1
    );
    let flushes_after_revision =
        engine.retained_stylist_flush_count_for_document_for_test(document);
    let resolutions_after_revision =
        engine.element_style_resolution_count_for_document_for_test(document);

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
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        1,
        "a clean read must not flush the retained style world again"
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        flushes_after_revision,
        "a clean read must not flush Stylo"
    );
    assert_eq!(
        engine.element_style_resolution_count_for_document_for_test(document),
        resolutions_after_revision,
        "a clean read must reuse the canonical ElementData style"
    );
}

#[test]
fn retained_stylesheet_resource_manifest_advances_only_when_resources_change() {
    reset_author_source_text_parse_count_for_test();
    reset_stylesheet_resource_manifest_build_count_for_test();
    let host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/resources.html").unwrap();
    let source_id = StyleSourceId::document_adopted_style_sheet(document, 71);
    let first_source = StyloStylesheetSource::new(
        "@import url(theme-a.css); @font-face { font-family: First; src: url(font-a.woff2); font-weight: 700; }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(source_id.clone()));
    engine.set_document_adopted_style_sheet_sources(document, vec![first_source.clone()]);
    let first_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![first_source],
        ..Default::default()
    };
    let first_key = StyleWorldKey::new(&first_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, first_key, &first_inputs);

    let first = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("the retained style world must publish its resource manifest");
    assert_eq!(first.web_fonts().len(), 1);
    assert_eq!(
        first.imports(),
        [url::Url::parse("https://example.test/theme-a.css").unwrap()]
    );
    assert_eq!(
        first.web_fonts()[0].request_url().as_str(),
        "https://example.test/font-a.woff2"
    );
    assert_eq!(author_source_text_parse_count_for_test(), 1);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 1);

    let clean = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("a clean world must retain its resource manifest");
    assert_eq!(clean.generation(), first.generation());
    assert_eq!(author_source_text_parse_count_for_test(), 1);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 1);

    let viewport = StyleViewport::from_width(Some(640.0));
    let viewport_key = StyleWorldKey::new(&first_inputs, viewport);
    engine.ensure_retained_style_system_for_document(&host, document, viewport_key, &first_inputs);
    let after_viewport_change = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("a device update must retain its resource manifest");
    assert_eq!(
        after_viewport_change.generation(),
        first.generation(),
        "device-only style updates must not advance the resource revision"
    );
    assert_eq!(
        stylesheet_resource_manifest_build_count_for_test(),
        2,
        "a device update must reproject effective resources without advancing an unchanged manifest"
    );

    let same_resources_source = StyloStylesheetSource::new(
        "@import url(theme-a.css); @font-face { font-family: First; src: url(font-a.woff2); font-weight: 700; } body { color: green; }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(source_id.clone()));
    engine.set_document_adopted_style_sheet_sources(document, vec![same_resources_source.clone()]);
    let same_resources_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![same_resources_source],
        ..Default::default()
    };
    let same_resources_key = StyleWorldKey::new(&same_resources_inputs, viewport);
    engine.ensure_retained_style_system_for_document(
        &host,
        document,
        same_resources_key,
        &same_resources_inputs,
    );
    let after_non_resource_change = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("a non-resource rule update must retain its resource manifest");
    assert_eq!(
        after_non_resource_change.generation(),
        first.generation(),
        "ordinary declaration changes must not restart resource reconciliation"
    );
    assert_eq!(author_source_text_parse_count_for_test(), 2);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 3);

    let second_source = StyloStylesheetSource::new(
        "@import url(theme-b.css); @font-face { font-family: Second; src: url(font-b.woff2); font-style: italic; }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(source_id));
    engine.set_document_adopted_style_sheet_sources(document, vec![second_source.clone()]);
    let second_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![second_source],
        ..Default::default()
    };
    let second_key = StyleWorldKey::new(&second_inputs, viewport);
    engine.ensure_retained_style_system_for_document(&host, document, second_key, &second_inputs);

    let second = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("the revised world must publish its new resource manifest");
    assert_ne!(second.generation(), first.generation());
    assert_eq!(second.web_fonts().len(), 1);
    assert_eq!(
        second.imports(),
        [url::Url::parse("https://example.test/theme-b.css").unwrap()]
    );
    assert_eq!(
        second.web_fonts()[0].request_url().as_str(),
        "https://example.test/font-b.woff2"
    );
    assert_eq!(
        author_source_text_parse_count_for_test(),
        3,
        "each stylesheet revision must be parsed once for both cascade and resources"
    );
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 4);
}

#[test]
fn retained_imported_font_resources_keep_each_import_parser_base_and_response_slot() {
    use style::{context::QuirksMode, stylesheets::AllowImportRules};

    let host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let registry = crate::live_stylesheet::LiveStylesheetRegistry::default();
    let root = registry.create(
        concat!(
            "@import '../theme/first/imported.css';",
            "@import '../theme/second/imported.css';",
            "@import '../redirect/entry.css';",
        ),
        url::Url::parse("https://example.test/css/root.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        engine.author_shared_lock(),
    );
    let responses = vec![
        crate::live_stylesheet::LiveStylesheetImportResponse {
            import_options: None,
            request_url: url::Url::parse("https://example.test/theme/first/imported.css").unwrap(),
            response_url: url::Url::parse("https://example.test/theme/first/imported.css").unwrap(),
            css_text: concat!(
                "@font-face { font-family: FirstImported; ",
                "src: url('./fonts/shared.woff2') format('woff2'); }",
            )
            .to_owned(),
            successful: true,
            origin_clean: true,
        },
        crate::live_stylesheet::LiveStylesheetImportResponse {
            import_options: None,
            request_url: url::Url::parse("https://example.test/theme/second/imported.css").unwrap(),
            response_url: url::Url::parse("https://example.test/theme/second/imported.css")
                .unwrap(),
            css_text: concat!(
                "@font-face { font-family: SecondImported; ",
                "src: url('./fonts/shared.woff2') format('woff2'); }",
            )
            .to_owned(),
            successful: true,
            origin_clean: true,
        },
        crate::live_stylesheet::LiveStylesheetImportResponse {
            import_options: None,
            request_url: url::Url::parse("https://example.test/redirect/entry.css").unwrap(),
            response_url: url::Url::parse("https://cdn.example.test/final/entry.css").unwrap(),
            css_text: "@import './nested.css';".to_owned(),
            successful: true,
            origin_clean: true,
        },
        crate::live_stylesheet::LiveStylesheetImportResponse {
            import_options: None,
            request_url: url::Url::parse("https://cdn.example.test/final/nested.css").unwrap(),
            response_url: url::Url::parse("https://assets.example.test/styles/nested.css").unwrap(),
            css_text: concat!(
                "@font-face { font-family: RedirectedNested; ",
                "src: local('RedirectedNested'), ",
                "url('../fonts/nested.svg') format('svg'), ",
                "url('../fonts/nested.woff2') format('woff2'); }",
            )
            .to_owned(),
            successful: true,
            origin_clean: true,
        },
    ];
    assert_eq!(
        registry.install_import_graph(
            root.id(),
            root.contents_revision(),
            root.import_generation(),
            &responses,
            Some(root.base_url()),
        ),
        Some(true),
        "the nested redirected import graph should install completely",
    );

    let source = StyloStylesheetSource::from_live_stylesheet(&root).with_source_id(Some(
        StyleSourceId::document_adopted_style_sheet(document, 73),
    ));
    engine.set_document_adopted_style_sheet_sources(document, vec![source.clone()]);
    let inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![source],
        ..Default::default()
    };
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);

    let snapshot = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("the imported font graph should publish a resource manifest");
    let retained_by_url = snapshot
        .web_fonts()
        .iter()
        .map(|resource| {
            (
                resource.request_url().as_str().to_owned(),
                resource
                    .web_font()
                    .expect("font manifest entry")
                    .slot()
                    .to_owned(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        retained_by_url.keys().cloned().collect::<Vec<_>>(),
        [
            "https://assets.example.test/fonts/nested.woff2".to_owned(),
            "https://example.test/theme/first/fonts/shared.woff2".to_owned(),
            "https://example.test/theme/second/fonts/shared.woff2".to_owned(),
        ],
        "every imported rule must retain its own response/parser base, including redirects",
    );
    assert_ne!(
        retained_by_url["https://example.test/theme/first/fonts/shared.woff2"],
        retained_by_url["https://example.test/theme/second/fonts/shared.woff2"],
        "the same relative src in different imported directories needs distinct slots",
    );

    for response in responses {
        for early in crate::css_resource_urls::stylesheet_load_blocking_font_resources(
            &response.css_text,
            &response.response_url,
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        ) {
            let retained_slot = &retained_by_url[early.request_url().as_str()];
            assert_eq!(
                retained_slot,
                early.web_font().expect("early response font").slot(),
                "response-time registration and retained reconciliation must share a slot",
            );
        }
    }
}

#[test]
fn retained_stylesheet_resource_manifest_tracks_effective_font_faces_across_media_changes() {
    reset_author_source_text_parse_count_for_test();
    reset_stylesheet_resource_manifest_build_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    assert!(host.append_child(document, shadow_host));
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("font resource fixture should create a ShadowRoot");
    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/media-fonts.html").unwrap();
    let owner_media_source = StyloStylesheetSource::new(
        "@font-face { font-family: OwnerPrint; src: url(owner-print.woff2); }".into(),
        document_url.clone(),
    )
    .with_owner_media_text("print");
    let nested_media_source = StyloStylesheetSource::new(
        "@media print { @font-face { font-family: NestedPrint; src: url(nested-print.woff2); } }\
         @media screen { @font-face { font-family: ScreenOnly; src: url(screen.woff2); } }"
            .into(),
        document_url,
    );
    let mut screen_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![owner_media_source, nested_media_source],
        ..Default::default()
    };
    screen_inputs.shadow_stylesheet_sources.push((
        shadow_root,
        vec![StyloStylesheetSource::new(
            "@media print { @font-face { font-family: ShadowPrint; src: url(shadow-print.woff2); } }\
             @media screen { @font-face { font-family: ShadowScreen; src: url(shadow-screen.woff2); } }"
                .into(),
            url::Url::parse("https://example.test/media-fonts.html").unwrap(),
        )],
    ));
    let screen_key = StyleWorldKey::new(&screen_inputs, None);
    engine.ensure_retained_style_system_for_document(
        &host,
        document,
        screen_key.clone(),
        &screen_inputs,
    );

    let resource_urls = |snapshot: &StylesheetResourceSnapshot| {
        let mut urls = snapshot
            .web_fonts()
            .iter()
            .map(|resource| resource.request_url().as_str().to_owned())
            .collect::<Vec<_>>();
        urls.sort();
        urls
    };
    let screen = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("screen media must publish an effective font projection");
    assert_eq!(
        resource_urls(&screen),
        [
            "https://example.test/screen.woff2",
            "https://example.test/shadow-screen.woff2",
        ]
    );
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 1);
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);

    let mut print_inputs = screen_inputs.clone();
    print_inputs.environment = StyloStyleEnvironment::from_emulated_media(
        &crate::protocol_types::EmulatedMediaOverrides {
            media: Some("print".to_owned()),
            ..Default::default()
        },
    );
    let print_key = StyleWorldKey::new(&print_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, print_key, &print_inputs);
    let print = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("print media must publish its effective font projection");
    assert_ne!(print.generation(), screen.generation());
    assert_eq!(
        resource_urls(&print),
        [
            "https://example.test/nested-print.woff2",
            "https://example.test/owner-print.woff2",
            "https://example.test/shadow-print.woff2",
        ]
    );
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "media changes must update the retained Stylist in place"
    );
    assert_eq!(
        author_source_text_parse_count_for_test(),
        3,
        "device projection must reuse parsed native rules"
    );

    engine.ensure_retained_style_system_for_document(&host, document, screen_key, &screen_inputs);
    let restored_screen = engine
        .stylesheet_resource_snapshot_for_document(document)
        .expect("returning to screen must publish the restored projection");
    assert_ne!(restored_screen.generation(), print.generation());
    assert_eq!(
        resource_urls(&restored_screen),
        [
            "https://example.test/screen.woff2",
            "https://example.test/shadow-screen.woff2",
        ]
    );
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity
    );
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    assert_eq!(stylesheet_resource_manifest_build_count_for_test(), 3);
}

#[test]
fn document_stylesheet_append_and_reorder_reuse_parsed_sheets() {
    reset_author_source_text_parse_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.append_child(document, target));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/ordered-sheets.html").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
        document, 61,
    )));
    let second_source = StyloStylesheetSource::new(
        ".target { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
        document, 62,
    )));

    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs
        .document_stylesheet_sources
        .push(first_source.clone());
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
    assert_eq!(author_source_text_parse_count_for_test(), 1);

    let appended_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![first_source.clone(), second_source.clone()],
        ..Default::default()
    };
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &appended_inputs,
            None,
        ),
        Some("rgb(4, 5, 6)".into())
    );
    assert_eq!(
        author_source_text_parse_count_for_test(),
        2,
        "appending one sheet must not reparse the existing sheet"
    );

    let reordered_inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: vec![second_source, first_source],
        ..Default::default()
    };
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "color",
            None,
            &reordered_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );
    assert_eq!(
        author_source_text_parse_count_for_test(),
        2,
        "reordering sheets must reuse both parsed stylesheet objects"
    );
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        2
    );
}

#[test]
fn incremental_document_stylesheet_updates_match_a_fresh_style_world_oracle() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("section");
    let child = host.create_element("span");
    let unrelated = host.create_element("aside");
    assert!(host.set_attribute(target, "class", "target"));
    assert!(host.set_attribute(unrelated, "class", "unrelated"));
    assert!(host.append_child(target, child));
    assert!(host.append_child(document, target));
    assert!(host.append_child(document, unrelated));

    let document_url = url::Url::parse("https://example.test/oracle.html").unwrap();
    let first_id = StyleSourceId::document_adopted_style_sheet(document, 81);
    let second_id = StyleSourceId::document_adopted_style_sheet(document, 82);
    let source = |css: &str, id: StyleSourceId| {
        StyloStylesheetSource::new(css.into(), document_url.clone()).with_source_id(Some(id))
    };
    let first = source(
        ".target { color: rgb(1, 2, 3); background-color: rgb(4, 5, 6); }",
        first_id.clone(),
    );
    let first_revision = source(
        ".target { color: rgb(7, 8, 9); background-color: rgb(10, 11, 12); }",
        first_id,
    );
    let second = source(
        ".target { color: rgb(13, 14, 15); } .unrelated { color: rgb(16, 17, 18); }",
        second_id,
    );
    let sequences = [
        vec![first.clone()],
        vec![first.clone(), second.clone()],
        vec![first_revision.clone(), second.clone()],
        vec![second.clone(), first_revision.clone()],
        vec![first_revision],
        Vec::new(),
    ];

    let mut incremental = MoliStyleEngine::new();
    for sources in sequences {
        incremental.set_document_adopted_style_sheet_sources(document, sources.clone());
        let inputs = FullStyleWorldSnapshot {
            document_stylesheet_sources: sources,
            ..Default::default()
        };
        let oracle = MoliStyleEngine::new();
        for (element, property) in [
            (target, "color"),
            (target, "background-color"),
            (child, "color"),
            (unrelated, "color"),
        ] {
            assert_eq!(
                incremental.computed_style_property_value(
                    &host,
                    &document_url,
                    element,
                    property,
                    None,
                    &inputs,
                    None,
                ),
                oracle.computed_style_property_value(
                    &host,
                    &document_url,
                    element,
                    property,
                    None,
                    &inputs,
                    None,
                ),
                "incremental and fresh worlds diverged for {element:?} {property}"
            );
        }
    }

    assert_eq!(
        incremental.retained_style_system_rebuild_count_for_document_for_test(document),
        1,
        "the oracle sequence must keep one persistent document Stylist"
    );
    assert_eq!(
        incremental.retained_style_system_update_count_for_document_for_test(document),
        5
    );
}

#[test]
fn one_shadow_stylesheet_change_preserves_the_other_scope_cascade_data() {
    reset_author_source_text_parse_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let first_host = host.create_element("section");
    let second_host = host.create_element("article");
    assert!(host.append_child(document, first_host));
    assert!(host.append_child(document, second_host));
    let first_root = host
        .attach_shadow_root(first_host, "open")
        .expect("first host should accept a shadow root");
    let second_root = host
        .attach_shadow_root(second_host, "open")
        .expect("second host should accept a shadow root");

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/shadow-incremental.html").unwrap();
    let first_source_id = StyleSourceId::shadow_root_adopted_style_sheet(first_root, 51);
    let second_source_id = StyleSourceId::shadow_root_adopted_style_sheet(second_root, 52);
    let first_source = StyloStylesheetSource::new(
        ":host { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(first_source_id.clone()));
    let second_source = StyloStylesheetSource::new(
        ":host { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(second_source_id));
    let mut first_inputs = FullStyleWorldSnapshot::default();
    first_inputs
        .shadow_stylesheet_sources
        .push((first_root, vec![first_source]));
    first_inputs
        .shadow_stylesheet_sources
        .push((second_root, vec![second_source.clone()]));
    let first_key = StyleWorldKey::new(&first_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, first_key, &first_inputs);

    let initial_data =
        engine.with_retained_style_system_for_document_for_test(document, |retained| {
            retained
                .shadow_cascade_data
                .iter()
                .map(|(root, data)| (*root, data.clone()))
                .collect::<Vec<_>>()
        });
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);
    let first_scope_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, first_root)
        .expect("first scope should be retained");
    let second_scope_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, second_root)
        .expect("second scope should be retained");
    assert_eq!(author_source_text_parse_count_for_test(), 2);

    let revised_first_source = StyloStylesheetSource::new(
        ":host { color: rgb(7, 8, 9); }".into(),
        document_url.clone(),
    )
    .with_source_id(Some(first_source_id));
    let mut second_inputs = FullStyleWorldSnapshot::default();
    second_inputs
        .shadow_stylesheet_sources
        .push((first_root, vec![revised_first_source]));
    second_inputs
        .shadow_stylesheet_sources
        .push((second_root, vec![second_source]));
    let second_key = StyleWorldKey::new(&second_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, second_key, &second_inputs);

    engine.with_retained_style_system_for_document_for_test(document, |retained| {
        let current_data = retained
            .shadow_cascade_data
            .iter()
            .map(|(root, data)| (*root, data))
            .collect::<Vec<_>>();
        assert!(!ServoArc::ptr_eq(&initial_data[0].1, current_data[0].1));
        assert!(ServoArc::ptr_eq(&initial_data[1].1, current_data[1].1));
    });
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, first_root),
        Some(first_scope_flushes + 1),
        "the dirty ShadowRoot must flush exactly once"
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, second_root),
        Some(second_scope_flushes),
        "an unrelated ShadowRoot must not flush"
    );
    assert_eq!(author_source_text_parse_count_for_test(), 3);
    assert_eq!(
        engine.retained_style_system_rebuild_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.retained_style_system_update_count_for_document_for_test(document),
        1
    );
}
