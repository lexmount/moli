use super::*;

#[test]
fn document_connected_shadow_scope_fallback_roots_include_shadow_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let active_host = host.create_element("section");
    let active_shadow_root = host
        .attach_shadow_root(active_host, "open")
        .expect("active section should host a shadow root");
    let detached_document = host.create_detached_html_document();
    let detached_host = host.create_element("section");
    let detached_shadow_root = host
        .attach_shadow_root(detached_host, "open")
        .expect("detached section should host a shadow root");

    assert!(host.append_child(document, active_host));
    assert!(host.append_child(detached_document, detached_host));

    let source_scope = StyleSourceScope::for_document_and_connected_shadow_roots(&host, document);
    let fallback_roots = source_scope_fallback_roots_for_test(&host, &source_scope);

    assert!(fallback_roots.contains(&document));
    assert!(fallback_roots.contains(&active_host));
    assert!(fallback_roots.contains(&active_shadow_root));
    assert!(!fallback_roots.contains(&detached_document));
    assert!(!fallback_roots.contains(&detached_host));
    assert!(!fallback_roots.contains(&detached_shadow_root));
}
#[test]
fn document_connected_shadow_source_scope_is_a_snapshot() {
    let mut host = test_host();
    let document = host.document_handle();
    let active_host = host.create_element("section");
    let active_shadow_root = host
        .attach_shadow_root(active_host, "open")
        .expect("active section should host a shadow root");
    let active_shadow_style = host.create_element("style");
    assert!(host.append_child(document, active_host));
    assert!(host.append_child(active_shadow_root, active_shadow_style));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        active_shadow_style,
        ".active { color: green; }".into(),
    );

    let source_scope = StyleSourceScope::for_document_and_connected_shadow_roots(&host, document);

    let late_host = host.create_element("article");
    let late_shadow_root = host
        .attach_shadow_root(late_host, "open")
        .expect("late article should host a shadow root");
    let late_shadow_style = host.create_element("style");
    assert!(host.append_child(document, late_host));
    assert!(host.append_child(late_shadow_root, late_shadow_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        late_shadow_style,
        ".late { color: red; }".into(),
    );

    let fallback_roots = source_scope_fallback_roots_for_test(&host, &source_scope);
    assert!(fallback_roots.contains(&document));
    assert!(fallback_roots.contains(&active_host));
    assert!(fallback_roots.contains(&active_shadow_root));
    assert!(!fallback_roots.contains(&late_host));
    assert!(!fallback_roots.contains(&late_shadow_root));

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let sources = engine.matching_dependency_source_ids_for_document_for_test(
        &host,
        document,
        &source_scope,
        &media,
    );
    let active_style_id = StyleSourceId {
        scope_id: StyleScopeId::ShadowRoot(active_shadow_root),
        kind: StyleSourceKind::OwnerStyleSheet {
            owner: active_shadow_style,
        },
    };
    let late_style_id = StyleSourceId {
        scope_id: StyleScopeId::ShadowRoot(late_shadow_root),
        kind: StyleSourceKind::OwnerStyleSheet {
            owner: late_shadow_style,
        },
    };

    assert!(
        sources
            .iter()
            .any(|(source_id, _)| source_id == &active_style_id),
        "snapshot should include active shadow source; sources={sources:?}"
    );
    assert!(
        !sources
            .iter()
            .any(|(source_id, _)| source_id == &late_style_id),
        "snapshot should not include shadow sources connected after scope creation; sources={sources:?}"
    );
}

#[test]
fn matching_dependency_sources_are_scoped_to_invalidation_document() {
    let mut host = test_host();
    let document = host.document_handle();
    let active_style = host.create_element("style");
    let active_target = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_style = host.create_element("style");
    let detached_target = host.create_element("main");
    assert!(host.append_child(document, active_style));
    assert!(host.append_child(document, active_target));
    assert!(host.append_child(detached_document, detached_style));
    assert!(host.append_child(detached_document, detached_target));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        active_style,
        "main.active { color: green; }".into(),
    );
    engine.set_owner_style_sheet_text_with_host(
        &host,
        detached_style,
        "main.detached { color: red; }".into(),
    );

    let source_scope = StyleSourceScope::for_handles(&host, [active_target, detached_target]);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let active_style_id =
        StyleSourceId::owner_style_sheet(&host, active_style).expect("active source id");
    let detached_style_id =
        StyleSourceId::owner_style_sheet(&host, detached_style).expect("detached source id");

    let active_sources = engine.matching_dependency_source_ids_for_document_for_test(
        &host,
        document,
        &source_scope,
        &media,
    );
    assert!(
        active_sources
            .iter()
            .any(|(source_id, _)| source_id == &active_style_id),
        "active document matching should keep the active document source"
    );
    assert!(
        !active_sources
            .iter()
            .any(|(source_id, _)| source_id == &detached_style_id),
        "active document matching must not scan detached document sources"
    );

    let detached_sources = engine.matching_dependency_source_ids_for_document_for_test(
        &host,
        detached_document,
        &source_scope,
        &media,
    );
    assert!(
        detached_sources
            .iter()
            .any(|(source_id, _)| source_id == &detached_style_id),
        "detached document matching should keep the detached document source"
    );
    assert!(
        !detached_sources
            .iter()
            .any(|(source_id, _)| source_id == &active_style_id),
        "detached document matching must not scan active document sources"
    );
}

#[test]
fn active_document_light_tree_mutation_matches_related_shadow_sources() {
    let mut host = test_host();
    let document = host.document_handle();
    let descendant_host = host.create_element("article");
    let descendant_shadow_root = host
        .attach_shadow_root(descendant_host, "open")
        .expect("descendant article should host a shadow root");
    let descendant_shadow_style = host.create_element("style");
    let descendant_target = host.create_element("span");
    let light_child = host.create_element("em");
    let sibling_host = host.create_element("aside");
    let sibling_shadow_root = host
        .attach_shadow_root(sibling_host, "open")
        .expect("sibling aside should host a shadow root");
    let sibling_shadow_style = host.create_element("style");
    let sibling_target = host.create_element("span");

    assert!(host.append_child(document, descendant_host));
    assert!(host.append_child(descendant_host, light_child));
    assert!(host.append_child(descendant_shadow_root, descendant_shadow_style));
    assert!(host.append_child(descendant_shadow_root, descendant_target));
    assert!(host.append_child(document, sibling_host));
    assert!(host.append_child(sibling_shadow_root, sibling_shadow_style));
    assert!(host.append_child(sibling_shadow_root, sibling_target));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        descendant_shadow_style,
        ":host:has(.marker) span { color: green; }".into(),
    );
    engine.set_owner_style_sheet_text_with_host(
        &host,
        sibling_shadow_style,
        ":host:has(.marker) span { color: red; }".into(),
    );

    let effect = [StyleMutationEffect::Attribute {
        element: light_child,
        name: "class".into(),
        old_value: None,
        new_value: Some("marker".into()),
    }];
    let source_scope =
        source_scope_for_mutations(&host, &effect).expect("connected mutation scope");
    let fallback_roots = source_scope_fallback_roots_for_test(&host, &source_scope);
    assert!(fallback_roots.contains(&document));
    assert!(fallback_roots.contains(&descendant_host));
    assert!(fallback_roots.contains(&descendant_shadow_root));
    assert!(!fallback_roots.contains(&sibling_host));
    assert!(!fallback_roots.contains(&sibling_shadow_root));

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let sources = engine.matching_dependency_source_ids_for_document_for_test(
        &host,
        document,
        &source_scope,
        &media,
    );
    let descendant_style_id = StyleSourceId {
        scope_id: StyleScopeId::ShadowRoot(descendant_shadow_root),
        kind: StyleSourceKind::OwnerStyleSheet {
            owner: descendant_shadow_style,
        },
    };
    let sibling_style_id = StyleSourceId {
        scope_id: StyleScopeId::ShadowRoot(sibling_shadow_root),
        kind: StyleSourceKind::OwnerStyleSheet {
            owner: sibling_shadow_style,
        },
    };

    assert!(
        sources
            .iter()
            .any(|(source_id, _)| source_id == &descendant_style_id),
        "light-tree mutation should include related shadow sources; sources={sources:?}"
    );
    assert!(
        !sources
            .iter()
            .any(|(source_id, _)| source_id == &sibling_style_id),
        "light-tree mutation should not include sibling shadow sources; sources={sources:?}"
    );
    let target_queries = {
        let world = engine.world_for_document(document);
        let linked_sources = world.linked_stylesheet_sources.borrow();
        let owner_sources = world.owner_style_sheet_sources.borrow();
        let adopted_sources = world.adopted_style_sheet_sources.borrow();
        super::planner::target_queries_for_pending_cause_with_adopted_sources(
            &host,
            &linked_sources,
            &owner_sources,
            &adopted_sources,
            &engine.dom_adapter,
            &media,
            StyleViewport::default(),
            host.document_handle(),
            &PendingStyleInvalidationCause::Mutation(effect.to_vec()),
            &source_scope,
        )
    };
    assert!(
        target_queries
            .iter()
            .any(|target_query| target_query.target().stylesheet_source_id()
                == Some(&descendant_style_id)),
        "light-tree mutation should produce target queries for related shadow source; target_queries={target_queries:?}"
    );
    assert!(
        !target_queries
            .iter()
            .any(|target_query| target_query.target().stylesheet_source_id()
                == Some(&sibling_style_id)),
        "light-tree mutation should not produce target queries for sibling shadow source; target_queries={target_queries:?}"
    );

    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    for handle in [descendant_target, sibling_target] {
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

    engine.invalidate_for_mutations(&host, &effect, &media);
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            document,
            descendant_target
        ),
        "the related shadow target stays published behind a lazy dirty root"
    );
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(document, sibling_target)
    );
    assert!(engine.retained_style_invalidation_root_count_for_document_for_test(document) > 0);
}
#[test]
fn author_stylesheet_sources_are_owned_by_style_engine() {
    let mut host = test_host();
    let mut engine = MoliStyleEngine::new();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let owner = host.create_element("style");
    assert!(host.append_child(document, shadow_host));
    assert!(host.append_child(document, owner));
    let url = url::Url::parse("https://example.test/app.css").unwrap();
    let redirected_url = url::Url::parse("https://cdn.example.test/assets/app.css").unwrap();
    let adopted_url = url::Url::parse("https://example.test/adopted.css").unwrap();
    let shadow_url = url::Url::parse("https://example.test/shadow.css").unwrap();

    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            "body { color: red; }".into(),
            adopted_url.clone(),
        )],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![StyloStylesheetSource::new(
            ":host { display: block; }".into(),
            shadow_url.clone(),
        )],
    );
    engine.set_owner_style_sheet_text_with_host(&host, owner, ".owner { color: blue; }".into());
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &url,
        StyloStylesheetSource::new(".linked { color: green; }".into(), redirected_url.clone()),
    );

    assert_eq!(
        engine.adopted_style_sheet_sources_for_document(document)[0]
            .serialized_css_text()
            .as_ref(),
        "body { color: red; }"
    );
    assert_eq!(
        engine.adopted_style_sheet_sources_for_document(document)[0].base_url(),
        &adopted_url
    );
    assert_eq!(
        engine.shadow_root_adopted_style_sheet_sources_with_host(&host, shadow_root)[0]
            .serialized_css_text()
            .as_ref(),
        ":host { display: block; }"
    );
    assert_eq!(
        engine.shadow_root_adopted_style_sheet_sources_with_host(&host, shadow_root)[0].base_url(),
        &shadow_url
    );
    assert!(
        engine.shadow_root_adopted_style_sheet_tracks_root_for_document_for_test(
            document,
            shadow_root
        )
    );
    assert_eq!(
        engine
            .owner_style_sheet_text_with_host(&host, owner)
            .as_deref(),
        Some(".owner { color: blue; }")
    );
    assert_eq!(
        engine
            .stylesheet_text_for_url_for_document_for_test(document, &url)
            .as_deref(),
        Some(".linked { color: green; }")
    );
    assert_eq!(
        engine
            .stylesheet_source_for_url_for_document_for_test(document, &url)
            .unwrap()
            .base_url(),
        &redirected_url
    );
    assert_eq!(
        engine
            .stylesheet_text_for_url_for_document_for_test(document, &redirected_url)
            .as_deref(),
        Some(".linked { color: green; }")
    );
    let request_source = engine
        .stylesheet_source_for_url_for_document_for_test(document, &url)
        .expect("request URL source");
    let final_source = engine
        .stylesheet_source_for_url_for_document_for_test(document, &redirected_url)
        .expect("final URL source");
    assert!(request_source.shares_source_storage_for_test(&final_source));
}

#[test]
fn redirected_linked_stylesheet_source_update_drops_stale_final_url_alias() {
    let host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();
    let request_url = url::Url::parse("https://example.test/app.css").unwrap();
    let old_final_url = url::Url::parse("https://cdn.example.test/old.css").unwrap();
    let new_final_url = url::Url::parse("https://cdn.example.test/new.css").unwrap();

    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &request_url,
        StyloStylesheetSource::new(
            ".linked { color: rgb(1, 2, 3); }".into(),
            old_final_url.clone(),
        ),
    );
    assert_eq!(
        engine
            .stylesheet_text_for_url_for_document_for_test(document, &old_final_url)
            .as_deref(),
        Some(".linked { color: rgb(1, 2, 3); }")
    );

    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &request_url,
        StyloStylesheetSource::new(
            ".linked { color: rgb(4, 5, 6); }".into(),
            new_final_url.clone(),
        ),
    );

    assert_eq!(
        engine
            .stylesheet_text_for_url_for_document_for_test(document, &request_url)
            .as_deref(),
        Some(".linked { color: rgb(4, 5, 6); }")
    );
    assert_eq!(
        engine
            .stylesheet_text_for_url_for_document_for_test(document, &new_final_url)
            .as_deref(),
        Some(".linked { color: rgb(4, 5, 6); }")
    );
    assert_eq!(
        engine.stylesheet_text_for_url_for_document_for_test(document, &old_final_url),
        None
    );
}

#[test]
fn retained_sources_are_scoped_to_document_worlds() {
    let mut host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();

    let disconnected_style = host.create_element("style");
    engine.set_owner_style_sheet_text_with_host(
        &host,
        disconnected_style,
        ".disconnected { color: red; }".into(),
    );

    let disconnected_link = host.create_element("link");
    assert!(host.set_attribute(disconnected_link, "rel", "stylesheet"));
    assert!(host.set_attribute(disconnected_link, "href", "app.css"));
    let linked_url = url::Url::parse("https://example.test/app.css").unwrap();
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &linked_url,
        StyloStylesheetSource::new(".linked { color: blue; }".into(), linked_url.clone()),
    );
    assert!(engine.install_recorded_linked_stylesheet_source_for_test(
        &host,
        disconnected_link,
        &linked_url,
    ));

    let inputs = FullStyleWorldSnapshot::default();
    engine.ensure_retained_style_system_for_document(
        &host,
        host.document_handle(),
        StyleWorldKey::new(&inputs, None),
        &inputs,
    );
    engine.with_retained_style_system_for_document_for_test(host.document_handle(), |retained| {
        assert!(
            retained.source_cascade_data.is_empty(),
            "disconnected active-document owners should stay trackable without retained source data"
        );
    });

    let detached_document = host.create_detached_html_document();
    let detached_style = host.create_element("style");
    assert!(host.append_child(detached_document, detached_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        detached_style,
        ".detached { color: green; }".into(),
    );
    let detached_style_id =
        StyleSourceId::owner_style_sheet(&host, detached_style).expect("detached style source id");

    let detached_link = host.create_element("link");
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached_link, "href", "app.css"));
    assert!(host.append_child(detached_document, detached_link));
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(".linked { color: blue; }".into(), linked_url.clone()),
        &[detached_link],
    );
    let detached_link_id =
        StyleSourceId::linked_style_sheet(&host, detached_link).expect("detached link source id");

    engine.ensure_retained_style_system_for_document(
        &host,
        host.document_handle(),
        StyleWorldKey::new(&inputs, None),
        &inputs,
    );
    engine.with_retained_style_system_for_document_for_test(host.document_handle(), |retained| {
        assert!(
            retained.source_cascade_data.is_empty(),
            "active document retained source data should not include detached document sources"
        );
    });

    engine.ensure_retained_style_system_for_document(
        &host,
        detached_document,
        StyleWorldKey::new(&inputs, None),
        &inputs,
    );
    engine.with_retained_style_system_for_document_for_test(detached_document, |retained| {
        assert_eq!(retained.source_cascade_data.len(), 2);
        assert!(
            retained
                .source_cascade_data
                .contains_key(&detached_style_id),
            "detached document owner stylesheet should stay local to detached retained data"
        );
        assert!(
            retained.source_cascade_data.contains_key(&detached_link_id),
            "detached linked stylesheet should stay local to detached retained data"
        );
    });
}

#[test]
fn removed_detached_document_stylesheet_owners_are_not_retained_sources() {
    let mut host = test_host();
    let detached_document = host.create_detached_html_document();
    let mut engine = MoliStyleEngine::new();

    let detached_style = host.create_element("style");
    assert!(host.append_child(detached_document, detached_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        detached_style,
        ".detached-style { color: green; }".into(),
    );
    let detached_style_id =
        StyleSourceId::owner_style_sheet(&host, detached_style).expect("detached style source id");

    let detached_link = host.create_element("link");
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached_link, "href", "detached.css"));
    assert!(host.append_child(detached_document, detached_link));
    let detached_link_url = url::Url::parse("https://example.test/detached.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &detached_link_url,
        StyloStylesheetSource::new(
            ".detached-link { color: blue; }".into(),
            detached_link_url.clone(),
        ),
        &[detached_link],
    );
    let detached_link_id =
        StyleSourceId::linked_style_sheet(&host, detached_link).expect("detached link source id");

    let retained_before_removal = engine
        .retained_stylesheet_source_ids_for_document_for_test(&host, detached_document)
        .into_iter()
        .collect::<HashSet<_>>();
    assert_eq!(
        retained_before_removal,
        HashSet::from([detached_style_id.clone(), detached_link_id.clone()])
    );

    assert!(host.remove_child(detached_document, detached_style));
    assert!(host.remove_child(detached_document, detached_link));

    let retained_after_removal = engine
        .retained_stylesheet_source_ids_for_document_for_test(&host, detached_document)
        .into_iter()
        .collect::<HashSet<_>>();
    assert!(
        retained_after_removal.is_empty(),
        "removed detached-document stylesheet owners must not stay retained"
    );

    let report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        detached_document,
        StyleSourceDocumentContext::for_root_document(detached_document),
    );
    for owner in [
        StyleSourceLifecycleOwner::OwnerStyleSheet {
            owner: detached_style,
        },
        StyleSourceLifecycleOwner::LinkedStyleSheet {
            owner: detached_link,
        },
    ] {
        assert_eq!(
            lifecycle_availability_for(report.records(), owner),
            Some(&StyleSourceLifecycleAvailability::Unavailable {
                reason: StyleSourceLifecycleUnavailableReason::OwnerNotInDocumentTree,
            })
        );
    }
}

#[test]
fn owner_stylesheet_source_moves_to_current_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let detached_document = host.create_detached_html_document();
    let mut engine = MoliStyleEngine::new();

    let owner = host.create_element("style");
    assert!(host.append_child(document, owner));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        owner,
        ".moved-owner { color: green; }".into(),
    );
    let old_source_id =
        StyleSourceId::owner_style_sheet(&host, owner).expect("old owner stylesheet source id");
    assert_eq!(
        old_source_id.scope_id,
        StyleScopeId::Document(document),
        "initial owner stylesheet source should belong to the active document"
    );
    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![old_source_id.clone()]
    );

    assert!(host.append_child(detached_document, owner));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        owner,
        ".moved-owner { color: green; }".into(),
    );
    let new_source_id =
        StyleSourceId::owner_style_sheet(&host, owner).expect("new owner stylesheet source id");
    assert_eq!(
        new_source_id.scope_id,
        StyleScopeId::Document(detached_document),
        "moved owner stylesheet source should belong to its current owner document"
    );
    assert!(
        engine
            .retained_stylesheet_source_ids_for_document_for_test(&host, document)
            .is_empty(),
        "old document world must not retain the moved owner stylesheet source"
    );
    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, detached_document),
        vec![new_source_id.clone()]
    );

    let old_document_report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
    );
    assert_eq!(
        lifecycle_availability_for(
            old_document_report.records(),
            StyleSourceLifecycleOwner::OwnerStyleSheet { owner },
        ),
        None,
        "stale owner stylesheet source should be removed from the old document world"
    );

    let new_document_report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        detached_document,
        StyleSourceDocumentContext::for_root_document(detached_document),
    );
    assert_eq!(
        lifecycle_availability_for(
            new_document_report.records(),
            StyleSourceLifecycleOwner::OwnerStyleSheet { owner },
        ),
        Some(&StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: vec![new_source_id],
        })
    );
}

#[test]
fn removed_child_document_owner_stylesheet_is_not_retained_source() {
    let mut host = test_host();
    let document = host.document_handle();
    let child_document = host.create_detached_html_document();
    let document_context =
        OwnedStyleSourceDocumentContext::new(document).with_child_documents([child_document]);
    let mut engine = MoliStyleEngine::new();

    let child_style = host.create_element("style");
    assert!(host.append_child(child_document, child_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        child_style,
        ".child-style { color: green; }".into(),
    );
    let child_style_id =
        StyleSourceId::owner_style_sheet(&host, child_style).expect("child style source id");

    let retained_before_removal = engine
        .retained_stylesheet_source_ids_for_document_for_test(&host, child_document)
        .into_iter()
        .collect::<HashSet<_>>();
    assert_eq!(retained_before_removal, HashSet::from([child_style_id]));

    assert!(host.remove_child(child_document, child_style));

    let retained_after_removal = engine
        .retained_stylesheet_source_ids_for_document_for_test(&host, child_document)
        .into_iter()
        .collect::<HashSet<_>>();
    assert!(
        retained_after_removal.is_empty(),
        "removed child-document stylesheet owner must not stay retained"
    );

    let report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        child_document,
        document_context.as_ref(),
    );
    assert_eq!(
        lifecycle_document_kind_for(
            report.records(),
            StyleSourceLifecycleOwner::OwnerStyleSheet { owner: child_style },
        ),
        Some(StyleSourceDocumentKind::Child)
    );
    assert_eq!(
        lifecycle_availability_for(
            report.records(),
            StyleSourceLifecycleOwner::OwnerStyleSheet { owner: child_style },
        ),
        Some(&StyleSourceLifecycleAvailability::Unavailable {
            reason: StyleSourceLifecycleUnavailableReason::OwnerNotInDocumentTree,
        })
    );
}

#[test]
fn source_lifecycle_snapshot_distinguishes_trackable_and_retained_sources() {
    let mut host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();

    let connected_style = host.create_element("style");
    assert!(host.append_child(document, connected_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        connected_style,
        ".connected { color: green; }".into(),
    );

    let disconnected_style = host.create_element("style");
    engine.set_owner_style_sheet_text_with_host(
        &host,
        disconnected_style,
        ".disconnected { color: red; }".into(),
    );

    let detached_document = host.create_detached_html_document();
    let detached_style = host.create_element("style");
    assert!(host.append_child(detached_document, detached_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        detached_style,
        ".detached { color: blue; }".into(),
    );

    let inactive_shadow_host = host.create_element("section");
    let inactive_shadow_root = host
        .attach_shadow_root(inactive_shadow_host, "open")
        .expect("inactive shadow host should accept a shadow root");
    let inactive_shadow_style = host.create_element("style");
    assert!(host.append_child(inactive_shadow_root, inactive_shadow_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        inactive_shadow_style,
        ":host { color: purple; }".into(),
    );

    let connected_link = host.create_element("link");
    assert!(host.set_attribute(connected_link, "rel", "stylesheet"));
    assert!(host.set_attribute(connected_link, "href", "connected.css"));
    assert!(host.append_child(document, connected_link));
    let connected_link_url = url::Url::parse("https://example.test/connected.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &connected_link_url,
        StyloStylesheetSource::new(
            ".linked-connected { color: black; }".into(),
            connected_link_url.clone(),
        ),
        &[connected_link],
    );

    let disconnected_link = host.create_element("link");
    assert!(host.set_attribute(disconnected_link, "rel", "stylesheet"));
    assert!(host.set_attribute(disconnected_link, "href", "disconnected.css"));
    let disconnected_link_url = url::Url::parse("https://example.test/disconnected.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &disconnected_link_url,
        StyloStylesheetSource::new(
            ".linked-disconnected { color: gray; }".into(),
            disconnected_link_url.clone(),
        ),
        &[disconnected_link],
    );

    let pending_link = host.create_element("link");
    assert!(host.set_attribute(pending_link, "rel", "stylesheet"));
    assert!(host.set_attribute(pending_link, "href", "pending.css"));
    assert!(host.append_child(document, pending_link));

    let missing_href_link = host.create_element("link");
    assert!(host.set_attribute(missing_href_link, "rel", "stylesheet"));
    assert!(host.append_child(document, missing_href_link));

    let missing_document = NativeNodeId::new(90_011);
    let empty_detached_document = host.create_detached_html_document();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".document-adopted { color: orange; }".into(),
            url::Url::parse("https://example.test/document-adopted.css").unwrap(),
        )],
    );
    engine.set_document_adopted_style_sheet_sources(
        missing_document,
        vec![StyloStylesheetSource::new(
            ".missing-document-adopted { color: pink; }".into(),
            url::Url::parse("https://example.test/missing-document-adopted.css").unwrap(),
        )],
    );
    engine.set_document_adopted_style_sheet_sources(empty_detached_document, Vec::new());

    let connected_shadow_host = host.create_element("article");
    assert!(host.append_child(document, connected_shadow_host));
    let connected_shadow_root = host
        .attach_shadow_root(connected_shadow_host, "open")
        .expect("connected shadow host should accept a shadow root");
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        connected_shadow_root,
        vec![StyloStylesheetSource::new(
            ":host { color: teal; }".into(),
            url::Url::parse("https://example.test/shadow-adopted.css").unwrap(),
        )],
    );
    let empty_connected_shadow_host = host.create_element("aside");
    assert!(host.append_child(document, empty_connected_shadow_host));
    let empty_connected_shadow_root = host
        .attach_shadow_root(empty_connected_shadow_host, "open")
        .expect("connected empty shadow host should accept a shadow root");
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        empty_connected_shadow_root,
        Vec::new(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        inactive_shadow_root,
        vec![StyloStylesheetSource::new(
            ":host { color: maroon; }".into(),
            url::Url::parse("https://example.test/inactive-shadow-adopted.css").unwrap(),
        )],
    );
    let document_adopted_source_ids =
        engine.document_adopted_style_sheet_source_ids_for_test(document);
    let connected_shadow_adopted_source_ids =
        engine.shadow_root_adopted_style_sheet_source_ids_for_test(&host, connected_shadow_root);

    let document_context = StyleSourceDocumentContext::for_root_document(document);
    let root_report =
        source_lifecycle_report_for_document_for_test(&engine, &host, document, document_context);
    let detached_report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        detached_document,
        document_context,
    );
    let empty_detached_report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        empty_detached_document,
        document_context,
    );
    let missing_document_report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        missing_document,
        document_context,
    );
    let root_snapshot = root_report.snapshot();
    assert_eq!(root_snapshot.tracked_owner_style_sheet_count(), 3);
    assert_eq!(root_snapshot.retained_owner_style_sheet_source_count(), 1);
    assert_eq!(root_snapshot.tracked_linked_style_sheet_owner_count(), 2);
    assert_eq!(root_snapshot.retained_linked_style_sheet_source_count(), 1);
    assert_eq!(root_snapshot.retained_document_adopted_source_count(), 1);
    assert_eq!(root_snapshot.retained_shadow_adopted_source_count(), 1);
    assert_eq!(root_snapshot.retained_root_document_source_count(), 4);
    assert_eq!(root_report.records().len(), 9);
    let detached_snapshot = detached_report.snapshot();
    assert_eq!(detached_snapshot.tracked_owner_style_sheet_count(), 1);
    assert_eq!(
        detached_snapshot.retained_owner_style_sheet_source_count(),
        1
    );
    assert_eq!(
        detached_snapshot.retained_detached_document_source_count(),
        1
    );
    assert_eq!(detached_report.records().len(), 1);
    assert_eq!(empty_detached_report.records().len(), 1);
    assert_eq!(missing_document_report.records().len(), 1);

    let connected_style_source_id = StyleSourceId::owner_style_sheet(&host, connected_style)
        .expect("connected owner stylesheet id");
    assert_eq!(
        root_report
            .record_for_source_id_for_test(&connected_style_source_id)
            .map(|record| record.availability().clone()),
        Some(StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: vec![connected_style_source_id.clone()],
        })
    );
    assert_eq!(
        empty_detached_report
            .record_for_source_id_for_test(&StyleSourceId::document_adopted_style_sheet(
                empty_detached_document,
                0,
            ))
            .map(|record| record.availability().clone()),
        Some(StyleSourceLifecycleAvailability::AvailableWithoutSource {
            reason: StyleSourceLifecycleWithoutSourceReason::EmptyDocumentAdoptedStyleSheets,
        })
    );
    assert_eq!(
        root_report
            .record_for_source_id_for_test(&StyleSourceId::document_adopted_style_sheet(
                document,
                u64::MAX,
            ))
            .map(|record| record.availability().clone()),
        Some(StyleSourceLifecycleAvailability::AvailableWithoutSource {
            reason: StyleSourceLifecycleWithoutSourceReason::SourceIdMissing,
        })
    );

    let active_retained_source_ids = engine
        .retained_stylesheet_source_ids_for_document_for_test(&host, document)
        .into_iter()
        .collect::<HashSet<_>>();
    assert_eq!(
        active_retained_source_ids,
        HashSet::from([
            StyleSourceId::owner_style_sheet(&host, connected_style)
                .expect("connected owner stylesheet id"),
            StyleSourceId::linked_style_sheet(&host, connected_link)
                .expect("connected linked stylesheet id"),
            document_adopted_source_ids[0].clone(),
            connected_shadow_adopted_source_ids[0].clone(),
        ])
    );
    let detached_retained_source_ids = engine
        .retained_stylesheet_source_ids_for_document_for_test(&host, detached_document)
        .into_iter()
        .collect::<HashSet<_>>();
    assert_eq!(
        detached_retained_source_ids,
        HashSet::from([StyleSourceId::owner_style_sheet(&host, detached_style)
            .expect("detached owner stylesheet id"),])
    );

    assert_eq!(
        lifecycle_availability_for(
            root_report.records(),
            StyleSourceLifecycleOwner::OwnerStyleSheet {
                owner: connected_style
            },
        ),
        Some(&StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: vec![
                StyleSourceId::owner_style_sheet(&host, connected_style)
                    .expect("connected owner stylesheet id"),
            ],
        })
    );
    assert_eq!(
        lifecycle_availability_for(
            root_report.records(),
            StyleSourceLifecycleOwner::OwnerStyleSheet {
                owner: disconnected_style
            },
        ),
        Some(&StyleSourceLifecycleAvailability::Unavailable {
            reason: StyleSourceLifecycleUnavailableReason::OwnerNotInDocumentTree,
        })
    );
    assert_eq!(
        lifecycle_availability_for(
            root_report.records(),
            StyleSourceLifecycleOwner::LinkedStyleSheet {
                owner: pending_link
            },
        ),
        None
    );
    assert_eq!(
        lifecycle_availability_for(
            root_report.records(),
            StyleSourceLifecycleOwner::LinkedStyleSheet {
                owner: missing_href_link
            },
        ),
        None
    );
    assert_eq!(
        lifecycle_availability_for(
            missing_document_report.records(),
            StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets {
                document: missing_document,
            },
        ),
        Some(&StyleSourceLifecycleAvailability::Unavailable {
            reason: StyleSourceLifecycleUnavailableReason::MissingNode,
        })
    );
    assert_eq!(
        lifecycle_availability_for(
            root_report.records(),
            StyleSourceLifecycleOwner::ShadowRootAdoptedStyleSheets {
                root: empty_connected_shadow_root,
            },
        ),
        Some(&StyleSourceLifecycleAvailability::AvailableWithoutSource {
            reason: StyleSourceLifecycleWithoutSourceReason::EmptyShadowRootAdoptedStyleSheets,
        })
    );
}
#[test]
fn source_lifecycle_context_distinguishes_child_and_detached_documents_in_outcome() {
    let mut host = test_host();
    let mut engine = MoliStyleEngine::new();
    let child_document = host.create_detached_html_document();
    let detached_document = host.create_detached_html_document();
    let document = host.document_handle();
    let document_context =
        OwnedStyleSourceDocumentContext::new(document).with_child_documents([child_document]);
    engine.set_document_adopted_style_sheet_sources(
        child_document,
        vec![StyloStylesheetSource::new(
            ".child { color: green; }".into(),
            url::Url::parse("https://example.test/child.css").unwrap(),
        )],
    );
    engine.set_document_adopted_style_sheet_sources(
        detached_document,
        vec![StyloStylesheetSource::new(
            ".detached { color: blue; }".into(),
            url::Url::parse("https://example.test/detached.css").unwrap(),
        )],
    );

    let child_report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        child_document,
        document_context.as_ref(),
    );
    let detached_report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        detached_document,
        document_context.as_ref(),
    );
    assert_eq!(
        child_report
            .snapshot()
            .retained_child_document_source_count(),
        1
    );
    assert_eq!(
        detached_report
            .snapshot()
            .retained_detached_document_source_count(),
        1
    );
    assert_eq!(
        lifecycle_document_kind_for(
            child_report.records(),
            StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets {
                document: child_document,
            },
        ),
        Some(StyleSourceDocumentKind::Child)
    );
    assert_eq!(
        lifecycle_document_kind_for(
            detached_report.records(),
            StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets {
                document: detached_document,
            },
        ),
        Some(StyleSourceDocumentKind::Detached)
    );

    let child_source = engine
        .document_adopted_style_sheet_source_ids_for_test(child_document)
        .into_iter()
        .next()
        .expect("child document should retain an adopted stylesheet source");
    let detached_source = engine
        .document_adopted_style_sheet_source_ids_for_test(detached_document)
        .into_iter()
        .next()
        .expect("detached document should retain an adopted stylesheet source");
    let child_target_queries = vec![PendingStyleInvalidationTargetQueries::retained_source(
        child_source,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(child_document)]),
    )];
    let child_application = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        child_document,
        document_context.as_ref(),
        None,
        &child_target_queries,
        false,
    )
    .finalize(&host);

    assert_eq!(
        child_application
            .diagnostic_target_result_summary()
            .child_document_source_target_count(),
        1
    );
    assert_eq!(
        child_application
            .diagnostic_target_result_summary()
            .detached_document_source_target_count(),
        0
    );
    assert_eq!(
        child_application
            .diagnostic_target_results()
            .iter()
            .filter(|result| {
                result
                    .diagnostic_source_target_availability()
                    .and_then(|availability| availability.document_kind())
                    == Some(StyleSourceDocumentKind::Child)
            })
            .count(),
        1
    );

    let detached_target_queries = vec![PendingStyleInvalidationTargetQueries::retained_source(
        detached_source,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(detached_document)]),
    )];
    let detached_application = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        detached_document,
        document_context.as_ref(),
        None,
        &detached_target_queries,
        false,
    )
    .finalize(&host);

    assert_eq!(
        detached_application
            .diagnostic_target_result_summary()
            .child_document_source_target_count(),
        0
    );
    assert_eq!(
        detached_application
            .diagnostic_target_result_summary()
            .detached_document_source_target_count(),
        1
    );
    assert_eq!(
        detached_application
            .diagnostic_target_results()
            .iter()
            .filter(|result| {
                result
                    .diagnostic_source_target_availability()
                    .and_then(|availability| availability.document_kind())
                    == Some(StyleSourceDocumentKind::Detached)
            })
            .count(),
        1
    );
}

#[derive(Default)]
struct SourceLifecycleOwnerDetailsForTest {
    details: Vec<StyleSourceLifecycleOwnerDetailTrace>,
}

impl StyleSourceLifecycleOwnerDetailTraceSink for SourceLifecycleOwnerDetailsForTest {
    fn record_source_lifecycle_owner_detail_trace(
        &mut self,
        trace: StyleSourceLifecycleOwnerDetailTrace,
    ) {
        self.details.push(trace);
    }
}

#[test]
fn source_lifecycle_owner_detail_trace_records_owner_document_kind_and_availability() {
    let mut host = test_host();
    let document = host.document_handle();
    let child_document = host.create_detached_html_document();
    let detached_document = host.create_detached_html_document();
    let missing_document = NativeNodeId::new(91_001);
    let mut engine = MoliStyleEngine::new();
    let document_context =
        OwnedStyleSourceDocumentContext::new(document).with_child_documents([child_document]);

    let connected_style = host.create_element("style");
    assert!(host.append_child(document, connected_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        connected_style,
        ".connected { color: green; }".into(),
    );

    let disconnected_style = host.create_element("style");
    engine.set_owner_style_sheet_text_with_host(
        &host,
        disconnected_style,
        ".disconnected { color: red; }".into(),
    );

    engine.set_document_adopted_style_sheet_sources(
        child_document,
        vec![StyloStylesheetSource::new(
            ".child { color: blue; }".into(),
            url::Url::parse("https://example.test/child.css").unwrap(),
        )],
    );
    engine.set_document_adopted_style_sheet_sources(
        detached_document,
        vec![StyloStylesheetSource::new(
            ".detached { color: purple; }".into(),
            url::Url::parse("https://example.test/detached.css").unwrap(),
        )],
    );
    engine.set_document_adopted_style_sheet_sources(
        missing_document,
        vec![StyloStylesheetSource::new(
            ".missing { color: orange; }".into(),
            url::Url::parse("https://example.test/missing.css").unwrap(),
        )],
    );

    let mut sink = SourceLifecycleOwnerDetailsForTest::default();
    for document in [
        document,
        child_document,
        detached_document,
        missing_document,
    ] {
        source_lifecycle_report_for_document_for_test(
            &engine,
            &host,
            document,
            document_context.as_ref(),
        )
        .record_owner_detail_trace_into(&mut sink);
    }

    let connected_owner = StyleSourceLifecycleOwner::OwnerStyleSheet {
        owner: connected_style,
    };
    let connected_source_id =
        StyleSourceId::owner_style_sheet(&host, connected_style).expect("connected source id");
    let connected_detail =
        lifecycle_owner_detail_for(&sink.details, connected_owner).expect("connected owner detail");
    assert_eq!(
        connected_detail.document_kind(),
        Some(StyleSourceDocumentKind::Root)
    );
    assert_eq!(
        connected_detail.availability(),
        &StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: vec![connected_source_id],
        }
    );

    let disconnected_detail = lifecycle_owner_detail_for(
        &sink.details,
        StyleSourceLifecycleOwner::OwnerStyleSheet {
            owner: disconnected_style,
        },
    )
    .expect("disconnected owner detail");
    assert_eq!(
        disconnected_detail.document_kind(),
        Some(StyleSourceDocumentKind::Root)
    );
    assert_eq!(
        disconnected_detail.availability(),
        &StyleSourceLifecycleAvailability::Unavailable {
            reason: StyleSourceLifecycleUnavailableReason::OwnerNotInDocumentTree,
        }
    );

    let child_detail = lifecycle_owner_detail_for(
        &sink.details,
        StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets {
            document: child_document,
        },
    )
    .expect("child document adopted detail");
    assert_eq!(
        child_detail.document_kind(),
        Some(StyleSourceDocumentKind::Child)
    );
    assert_eq!(
        child_detail.availability(),
        &StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: engine.document_adopted_style_sheet_source_ids_for_test(child_document),
        }
    );

    let detached_detail = lifecycle_owner_detail_for(
        &sink.details,
        StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets {
            document: detached_document,
        },
    )
    .expect("detached document adopted detail");
    assert_eq!(
        detached_detail.document_kind(),
        Some(StyleSourceDocumentKind::Detached)
    );

    let missing_detail = lifecycle_owner_detail_for(
        &sink.details,
        StyleSourceLifecycleOwner::DocumentAdoptedStyleSheets {
            document: missing_document,
        },
    )
    .expect("missing document adopted detail");
    assert_eq!(missing_detail.document_kind(), None);
    assert_eq!(
        missing_detail.availability(),
        &StyleSourceLifecycleAvailability::Unavailable {
            reason: StyleSourceLifecycleUnavailableReason::MissingNode,
        }
    );
}

#[test]
fn source_lifecycle_report_for_source_ids_only_records_requested_tracked_owners() {
    let mut host = test_host();
    let mut engine = MoliStyleEngine::new();
    let document = host.document_handle();

    let requested_style = host.create_element("style");
    assert!(host.append_child(document, requested_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        requested_style,
        ".requested { color: green; }".into(),
    );
    let requested_source_id =
        StyleSourceId::owner_style_sheet(&host, requested_style).expect("requested source id");

    let unrelated_style = host.create_element("style");
    assert!(host.append_child(document, unrelated_style));
    engine.set_owner_style_sheet_text_with_host(
        &host,
        unrelated_style,
        ".unrelated { color: blue; }".into(),
    );
    let unrelated_source_id =
        StyleSourceId::owner_style_sheet(&host, unrelated_style).expect("unrelated source id");

    let report = {
        let document_world = engine.world_for_document(document);
        let source_stores = document_world.borrow_source_stores();
        source_stores.source_lifecycle_report_for_source_ids(
            &host,
            StyleSourceDocumentContext::for_root_document(document),
            [requested_source_id.clone(), requested_source_id.clone()],
        )
    };

    assert_eq!(report.records().len(), 1);
    assert_eq!(
        report
            .record_for_source_id_for_test(&requested_source_id)
            .map(|record| record.availability().clone()),
        Some(StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: vec![requested_source_id],
        })
    );
    assert!(
        report
            .record_for_source_id_for_test(&unrelated_source_id)
            .is_none()
    );
    assert_eq!(report.snapshot().tracked_owner_style_sheet_count(), 1);
    assert_eq!(
        report.snapshot().retained_owner_style_sheet_source_count(),
        1
    );

    let adopted_base_url = url::Url::parse("https://example.test/adopted.css").unwrap();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![
            StyloStylesheetSource::new(
                ".adopted-a { color: purple; }".into(),
                adopted_base_url.clone(),
            ),
            StyloStylesheetSource::new(".adopted-b { color: orange; }".into(), adopted_base_url),
        ],
    );
    let adopted_source_ids = engine.document_adopted_style_sheet_source_ids_for_test(document);
    let requested_adopted_source_id = adopted_source_ids[0].clone();
    let unrequested_adopted_source_id = adopted_source_ids[1].clone();
    let adopted_report = {
        let document_world = engine.world_for_document(document);
        let source_stores = document_world.borrow_source_stores();
        source_stores.source_lifecycle_report_for_source_ids(
            &host,
            StyleSourceDocumentContext::for_root_document(document),
            [requested_adopted_source_id.clone()],
        )
    };

    assert_eq!(
        adopted_report
            .record_for_source_id_for_test(&requested_adopted_source_id)
            .map(|record| record.availability().clone()),
        Some(StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: vec![requested_adopted_source_id],
        })
    );
    assert_eq!(
        adopted_report
            .record_for_source_id_for_test(&unrequested_adopted_source_id)
            .map(|record| record.availability().clone()),
        Some(StyleSourceLifecycleAvailability::AvailableWithoutSource {
            reason: StyleSourceLifecycleWithoutSourceReason::SourceIdMissing,
        })
    );
    assert_eq!(
        adopted_report
            .snapshot()
            .retained_document_adopted_source_count(),
        1
    );

    let linked = host.create_element("link");
    assert!(host.set_attribute(linked, "rel", "stylesheet"));
    assert!(host.set_attribute(linked, "href", "linked.css"));
    assert!(host.append_child(document, linked));
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(".linked { color: black; }".into(), linked_url.clone()),
        &[linked],
    );
    let requested_linked_source_id =
        StyleSourceId::linked_style_sheet(&host, linked).expect("linked source id");
    let linked_report = {
        let document_world = engine.world_for_document(document);
        let source_stores = document_world.borrow_source_stores();
        source_stores.source_lifecycle_report_for_source_ids(
            &host,
            StyleSourceDocumentContext::for_root_document(document),
            [requested_linked_source_id.clone()],
        )
    };

    assert_eq!(
        linked_report
            .record_for_source_id_for_test(&requested_linked_source_id)
            .map(|record| record.availability().clone()),
        Some(StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: vec![requested_linked_source_id.clone()],
        })
    );
    assert_eq!(
        lifecycle_availability_for(
            linked_report.records(),
            StyleSourceLifecycleOwner::LinkedStyleSheet { owner: linked },
        ),
        Some(&StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: vec![requested_linked_source_id.clone()],
        })
    );
    assert_eq!(
        linked_report
            .snapshot()
            .tracked_linked_style_sheet_owner_count(),
        1
    );
    assert_eq!(
        linked_report
            .snapshot()
            .retained_linked_style_sheet_source_count(),
        1
    );

    let detached_document = host.create_detached_html_document();
    let detached_link = host.create_element("link");
    assert!(host.set_attribute(detached_link, "rel", "stylesheet"));
    assert!(host.set_attribute(detached_link, "href", "linked.css"));
    assert!(host.append_child(detached_document, detached_link));
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(
            ".detached-linked { color: blue; }".into(),
            linked_url.clone(),
        ),
        &[detached_link],
    );
    let detached_linked_source_id =
        StyleSourceId::linked_style_sheet(&host, detached_link).expect("detached linked source id");
    let active_document_report_for_detached_source = {
        let document_world = engine.world_for_document(document);
        let source_stores = document_world.borrow_source_stores();
        source_stores.source_lifecycle_report_for_source_ids(
            &host,
            StyleSourceDocumentContext::for_root_document(document),
            [detached_linked_source_id.clone()],
        )
    };
    assert!(
        active_document_report_for_detached_source
            .record_for_source_id_for_test(&detached_linked_source_id)
            .is_none(),
        "linked source-id lifecycle must not bypass the current document owner registry"
    );
    let detached_document_report_for_detached_source = {
        let detached_world = engine.world_for_document(detached_document);
        let source_stores = detached_world.borrow_source_stores();
        source_stores.source_lifecycle_report_for_source_ids(
            &host,
            StyleSourceDocumentContext::for_root_document(detached_document),
            [detached_linked_source_id.clone()],
        )
    };
    assert_eq!(
        detached_document_report_for_detached_source
            .record_for_source_id_for_test(&detached_linked_source_id)
            .map(|record| record.availability().clone()),
        Some(StyleSourceLifecycleAvailability::RetainedSources {
            source_ids: vec![detached_linked_source_id.clone()],
        })
    );

    let untracked_document =
        host.create_document(url::Url::parse("https://example.test/untracked/").unwrap());
    let untracked_report = {
        let document_world = engine.world_for_document(document);
        let source_stores = document_world.borrow_source_stores();
        source_stores.source_lifecycle_report_for_source_ids(
            &host,
            StyleSourceDocumentContext::for_root_document(document),
            [StyleSourceId::document_adopted_style_sheet(
                untracked_document,
                0,
            )],
        )
    };

    assert!(untracked_report.records().is_empty());
}

#[test]
fn owner_style_sheet_sources_share_one_cached_text_allocation() {
    let mut host = test_host();
    let first_owner = host.create_element("style");
    let second_owner = host.create_element("style");
    let mut owner_sources = super::source_owner_text::OwnerStyleSheetSources::default();
    let base_url = url::Url::parse("https://example.test/").expect("valid test url");
    let css_text = ".card:has(.item) { color: red; }";
    owner_sources.set_source(first_owner, css_text.to_owned(), base_url.clone());
    owner_sources.set_source(second_owner, css_text.to_owned(), base_url);

    let first = owner_sources
        .source(first_owner)
        .expect("owner stylesheet source");
    let second = owner_sources
        .source(second_owner)
        .expect("owner stylesheet source");
    let first_processing_source = owner_sources
        .processing_source(first_owner)
        .expect("first processing source");
    let source_text = first_processing_source
        .source()
        .input_css_text()
        .expect("owner processing source must remain text-backed");

    assert!(first.shares_source_storage_for_test(&second));
    assert_eq!(
        first_processing_source.css_text().as_ptr(),
        source_text.as_ptr()
    );
}

#[test]
fn identical_stylesheet_sources_share_immutable_source_storage() {
    let base_url = url::Url::parse("https://example.test/styles/").expect("valid test url");
    let first = StyloStylesheetSource::new(
        ".card:has(.item) { color: red; }".to_owned(),
        base_url.clone(),
    );
    let second =
        StyloStylesheetSource::new(".card:has(.item) { color: red; }".to_owned(), base_url);

    assert!(first.shares_source_storage_for_test(&second));
}

#[test]
fn stylesheet_source_storage_keeps_base_url_separate() {
    let first = StyloStylesheetSource::new(
        ".card { background: url(bg.png); }".to_owned(),
        url::Url::parse("https://example.test/a/").expect("valid test url"),
    );
    let second = StyloStylesheetSource::new(
        ".card { background: url(bg.png); }".to_owned(),
        url::Url::parse("https://example.test/b/").expect("valid test url"),
    );

    assert!(!first.shares_source_storage_for_test(&second));
}

#[test]
fn shared_source_storage_keeps_source_identity_out_of_shared_cache() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_style = host.create_element("style");
    let second_style = host.create_element("style");
    assert!(host.append_child(document, first_style));
    assert!(host.append_child(document, second_style));

    let base_url = url::Url::parse("https://example.test/").expect("valid test url");
    let css_text = ".shared { color: green; }";
    let first_source = StyloStylesheetSource::new(css_text.to_owned(), base_url.clone())
        .with_source_id(Some(
            StyleSourceId::owner_style_sheet(&host, first_style).expect("first source id"),
        ));
    let second_source = StyloStylesheetSource::new(css_text.to_owned(), base_url).with_source_id(
        Some(StyleSourceId::owner_style_sheet(&host, second_style).expect("second source id")),
    );

    assert!(first_source.shares_source_storage_for_test(&second_source));
    assert_ne!(
        super::source::store::stylesheet_sources_cache_key(&[first_source]),
        super::source::store::stylesheet_sources_cache_key(&[second_source])
    );
}

#[test]
fn linked_source_store_lifecycle_records_drive_retained_record_construction() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let link = host.create_element("link");
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, link));
    assert!(host.append_child(document, shadow_host));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "linked.css"));

    let mut linked_sources = super::source::linked::LinkedStylesheetSources::default();
    let mut owner_sources = super::source_owner_text::OwnerStyleSheetSources::default();
    owner_sources.set_source(
        style,
        ".owner { color: green; }".to_owned(),
        url::Url::parse("https://example.test/").unwrap(),
    );
    let linked_url = url::Url::parse("https://example.test/linked.css").unwrap();
    linked_sources.record_source_for_url(
        &linked_url,
        StyloStylesheetSource::new(".linked { color: blue; }".into(), linked_url.clone()),
    );
    assert!(linked_sources.bind_owner(link, linked_url));
    let mut adopted_sources = super::source::adopted::AdoptedStyleSheetSources::default();
    adopted_sources.set_document_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".document { color: purple; }".into(),
            url::Url::parse("https://example.test/document.css").unwrap(),
        )],
    );
    adopted_sources.set_shadow_root_sources(
        shadow_root,
        vec![StyloStylesheetSource::new(
            ":host { color: orange; }".into(),
            url::Url::parse("https://example.test/shadow.css").unwrap(),
        )],
    );
    let source_stores = super::source_document::DocumentStyleSourceStores::borrowed_for_test(
        document,
        &linked_sources,
        &owner_sources,
        &adopted_sources,
    );
    let report = source_stores.source_lifecycle_report(
        &host,
        StyleSourceDocumentContext::for_root_document(document),
    );
    let retained_records = source_stores.retained_source_records_for_lifecycle(&host, &report);
    let retained_record_ids = retained_records
        .into_iter()
        .map(|record| record.id().clone())
        .collect::<HashSet<_>>();
    let document_adopted_source_id = adopted_sources
        .document_source_ids(document)
        .into_iter()
        .next()
        .expect("document adopted source id");
    let shadow_adopted_source_id = adopted_sources
        .shadow_root_source_ids(shadow_root)
        .into_iter()
        .next()
        .expect("shadow adopted source id");

    assert_eq!(
        retained_record_ids,
        HashSet::from([
            StyleSourceId::owner_style_sheet(&host, style).expect("owner source id"),
            StyleSourceId::linked_style_sheet(&host, link).expect("linked source id"),
            document_adopted_source_id,
            shadow_adopted_source_id,
        ])
    );
}

#[test]
fn linked_owner_binding_requires_explicit_source_installation() {
    let mut host = test_host();
    let document = host.document_handle();
    let link = host.create_element("link");
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "old.css"));

    let mut engine = MoliStyleEngine::new();
    let new_url = url::Url::parse("https://example.test/new.css").unwrap();
    assert!(host.set_attribute(link, "href", "new.css"));
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &new_url,
        StyloStylesheetSource::new(
            "section { color: rgb(1, 2, 3); }".to_owned(),
            new_url.clone(),
        ),
    );

    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        Vec::<StyleSourceId>::new()
    );

    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &new_url,
        StyloStylesheetSource::new("section { color: green; }".to_owned(), new_url.clone()),
        &[link],
    );

    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![StyleSourceId::linked_style_sheet(&host, link).expect("linked source id")]
    );
}

#[test]
fn inline_owner_reprocessing_clears_the_previous_import_source_binding() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    assert!(host.set_text_content(style, "@import url('old.css');"));
    assert!(host.append_child(document, style));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(&host, style, "@import url('old.css');".to_owned());
    let old_url = url::Url::parse("https://example.test/old.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &old_url,
        StyloStylesheetSource::new(".old { color: red; }".to_owned(), old_url.clone()),
        &[style],
    );
    assert!(
        engine
            .linked_stylesheet_source_for_owner_with_host(&host, style)
            .is_some()
    );

    let effects = host.set_text_content_effects(style, ".next { color: blue; }");
    engine.apply_stylesheet_owner_changes_with_host(&host, effects.stylesheet_owners().changes());

    assert!(
        engine
            .linked_stylesheet_source_for_owner_with_host(&host, style)
            .is_none(),
        "reprocessing an inline owner must detach the imported source installed for the previous processing object"
    );
    assert_eq!(
        engine.owner_style_sheet_text_with_host(&host, style),
        Some(".next { color: blue; }".to_owned())
    );
}

#[test]
fn reconnecting_inline_owner_creates_a_new_processing_source_identity() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    assert!(host.set_text_content(style, "@import url('same.css');"));
    assert!(host.append_child(document, style));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "@import url('same.css');".to_owned(),
    );
    let first = engine
        .owner_style_sheet_processing_source(style)
        .expect("initial processing source");

    let removal = host.remove_child_effects(document, style);
    engine.apply_stylesheet_owner_changes_with_host(&host, removal.stylesheet_owners().changes());
    let insertion = host.append_child_effects(document, style);
    engine.apply_stylesheet_owner_changes_with_host(&host, insertion.stylesheet_owners().changes());
    let second = engine
        .owner_style_sheet_processing_source(style)
        .expect("reconnected processing source");

    assert!(
        !std::sync::Arc::ptr_eq(&first, &second),
        "a real reconnect processing step must create a fresh operation identity even when text and base are unchanged"
    );
}

#[test]
fn disconnecting_inline_owner_retires_source_and_advances_document_generation() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    assert!(host.set_text_content(style, "body { color: green; }"));
    assert!(host.append_child(document, style));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(&host, style, "body { color: green; }".to_owned());
    let generation = engine.source_set_generation_for_document_for_test(document);
    assert!(engine.owner_style_sheet_processing_source(style).is_some());

    let removal = host.remove_child_effects(document, style);
    engine.apply_stylesheet_owner_changes_with_host(&host, removal.stylesheet_owners().changes());

    assert!(
        engine.owner_style_sheet_processing_source(style).is_none(),
        "a disconnected owner must not remain installed in its old Document world"
    );
    assert!(
        engine.source_set_generation_for_document_for_test(document) > generation,
        "retiring the owner must make retained style and resource manifests observably dirty"
    );
}

#[test]
fn linked_owner_binding_changes_only_on_explicit_install() {
    let mut host = test_host();
    let document = host.document_handle();
    let link = host.create_element("link");
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "old.css"));

    let old_url = url::Url::parse("https://example.test/old.css").unwrap();
    let new_url = url::Url::parse("https://example.test/new.css").unwrap();
    let mut engine = MoliStyleEngine::new();
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &old_url,
        StyloStylesheetSource::new(".old { color: red; }".into(), old_url.clone()),
    );
    assert!(engine.install_recorded_linked_stylesheet_source_for_test(&host, link, &old_url,));

    let linked_source_id =
        StyleSourceId::linked_style_sheet(&host, link).expect("linked source id");
    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![linked_source_id.clone()]
    );

    let effects = host.set_attribute_effects(link, "href", "new.css");
    engine.apply_stylesheet_owner_changes_with_host(&host, effects.stylesheet_owners().changes());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);

    assert!(
        engine
            .retained_stylesheet_source_ids_for_document_for_test(&host, document)
            .is_empty(),
        "the old loaded stylesheet must not remain bound after href changes"
    );

    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &new_url,
        StyloStylesheetSource::new(".new { color: blue; }".into(), new_url.clone()),
    );
    assert!(
        engine
            .retained_stylesheet_source_ids_for_document_for_test(&host, document)
            .is_empty(),
        "recording a resource by URL must not infer an owner binding from live href"
    );
    assert!(engine.install_recorded_linked_stylesheet_source_for_test(&host, link, &new_url,));
    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![linked_source_id]
    );
}

#[test]
fn linked_owner_binding_survives_media_update_without_reinstall() {
    let mut host = test_host();
    let document = host.document_handle();
    let link = host.create_element("link");
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "app.css"));
    assert!(host.set_attribute(link, "media", "print"));

    let request_url = url::Url::parse("https://example.test/app.css").unwrap();
    let mut engine = MoliStyleEngine::new();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &request_url,
        StyloStylesheetSource::new("main { color: green; }".into(), request_url.clone()),
        &[link],
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1)
    );

    let effects = host.set_attribute_effects(link, "media", "screen");
    engine.apply_stylesheet_owner_changes_with_host(&host, effects.stylesheet_owners().changes());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);

    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1),
        "media changes update the installed sheet instead of detaching its load/source binding"
    );
    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![StyleSourceId::linked_style_sheet(&host, link).expect("linked source id")]
    );
}

#[test]
fn linked_owner_keeps_processed_source_when_base_href_changes() {
    let mut host = test_host();
    let document = host.document_handle();
    let base = host.create_element("base");
    assert!(host.set_attribute(base, "href", "/assets/"));
    assert!(host.append_child(document, base));
    let link = host.create_element("link");
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "app.css"));

    let old_url = url::Url::parse("https://example.test/assets/app.css").unwrap();
    let mut engine = MoliStyleEngine::new();
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &old_url,
        StyloStylesheetSource::new(".old-base { color: red; }".into(), old_url.clone()),
    );
    assert!(engine.install_recorded_linked_stylesheet_source_for_test(&host, link, &old_url,));

    let linked_source_id =
        StyleSourceId::linked_style_sheet(&host, link).expect("linked source id");
    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![linked_source_id.clone()]
    );

    assert!(host.set_attribute(base, "href", "/theme/"));
    let effects = [StyleMutationEffect::Attribute {
        element: base,
        name: "href".into(),
        old_value: Some("/assets/".into()),
        new_value: Some("/theme/".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);

    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![linked_source_id],
        "changing the document base must not re-key an already processed link"
    );
}

#[test]
fn linked_owner_keeps_processed_source_when_base_href_is_inserted() {
    let mut host = test_host();
    let document = host.document_handle();
    let link = host.create_element("link");
    assert!(host.append_child(document, link));
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "app.css"));

    let old_url = url::Url::parse("https://example.test/app.css").unwrap();
    let mut engine = MoliStyleEngine::new();
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &old_url,
        StyloStylesheetSource::new(".old-base { color: red; }".into(), old_url.clone()),
    );
    assert!(engine.install_recorded_linked_stylesheet_source_for_test(&host, link, &old_url,));

    let linked_source_id =
        StyleSourceId::linked_style_sheet(&host, link).expect("linked source id");
    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![linked_source_id.clone()]
    );

    let base = host.create_element("base");
    assert!(host.set_attribute(base, "href", "/assets/"));
    assert!(host.insert_before(document, base, Some(link)));
    let effects = [StyleMutationEffect::ChildList {
        parent: document,
        added_nodes: vec![base],
        removed_nodes: vec![],
        removed_element_snapshots: vec![],
        previous_sibling: None,
        next_sibling: Some(link),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);

    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![linked_source_id],
        "inserting a document base must not re-key an already processed link"
    );
}

#[test]
fn source_lifecycle_target_result_reports_missing_adopted_source_id() {
    let host = test_host();
    let mut engine = MoliStyleEngine::new();
    let document = host.document_handle();
    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            ".existing { color: green; }".into(),
            url::Url::parse("https://example.test/existing.css").unwrap(),
        )],
    );
    let missing_source_id = StyleSourceId::document_adopted_style_sheet(document, u64::MAX);
    let target_query = PendingStyleInvalidationTargetQueries::retained_source(
        missing_source_id,
        indexmap::IndexSet::from([RetainedStyleInvalidationQuery::universal(document)]),
    );

    let outcome = retained_source_invalidation_outcome_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        None,
        std::slice::from_ref(&target_query),
        false,
    );
    let application = outcome.finalize(&host);

    assert_eq!(application.diagnostic_target_results().len(), 1);
    assert_eq!(
        application.diagnostic_target_results()[0]
            .diagnostic_source_target_availability()
            .and_then(|availability| availability.lifecycle().cloned()),
        Some(StyleSourceLifecycleAvailability::AvailableWithoutSource {
            reason: StyleSourceLifecycleWithoutSourceReason::SourceIdMissing,
        })
    );
    assert_eq!(
        application
            .diagnostic_target_result_summary()
            .source_lifecycle_available_without_source_target_count(),
        1
    );
}
#[test]
fn missing_document_adopted_sources_remain_trackable_without_retained_source() {
    let host = test_host();
    let mut engine = MoliStyleEngine::new();
    let missing_document = NativeNodeId::new(90_001);
    let source_url = url::Url::parse("https://example.test/missing-document.css").unwrap();
    let source_id = StyleSourceId::document_adopted_style_sheet(missing_document, 0);

    engine.set_document_adopted_style_sheet_sources(
        missing_document,
        vec![StyloStylesheetSource::new(
            "body:has(.marker) .target { color: green; }".into(),
            source_url,
        )],
    );

    assert_eq!(
        engine.adopted_style_sheet_source_owner_counts_for_document_for_test(missing_document),
        (1, 0),
        "missing document owner stays trackable for future wrapper sync"
    );
    assert!(engine.document_adopted_style_sheet_tracks_document_for_test(missing_document));

    let source_scope = StyleSourceScope::for_document(host.document_handle());
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    assert!(
        engine
            .matching_dependency_source_ids_for_document_for_test(
                &host,
                host.document_handle(),
                &source_scope,
                &media,
            )
            .is_empty(),
        "missing document adopted sources should not participate in dependency matching"
    );

    let inputs = FullStyleWorldSnapshot::default();
    engine.ensure_retained_style_system_for_document(
        &host,
        host.document_handle(),
        StyleWorldKey::new(&inputs, None),
        &inputs,
    );
    engine.with_retained_style_system_for_document_for_test(host.document_handle(), |retained| {
        assert!(
            !retained.source_cascade_data.contains_key(&source_id),
            "missing document adopted sources should not produce retained cascade data"
        );
    });
}
#[test]
fn unchanged_stylesheet_source_syncs_do_not_advance_generation() {
    let mut host = test_host();
    let mut engine = MoliStyleEngine::new();
    let active_document = host.document_handle();
    let document = active_document;
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let owner = host.create_element("style");
    assert!(host.append_child(active_document, shadow_host));
    assert!(host.append_child(active_document, owner));
    let url = url::Url::parse("https://example.test/app.css").unwrap();
    let document_url = url::Url::parse("https://example.test/document.css").unwrap();
    let shadow_url = url::Url::parse("https://example.test/shadow.css").unwrap();
    let linked_url = url::Url::parse("https://cdn.example.test/app.css").unwrap();

    engine.set_document_adopted_style_sheet_sources(document, Vec::new());
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(&host, shadow_root, Vec::new());
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(active_document),
        0
    );
    assert!(engine.document_adopted_style_sheet_tracks_document_for_test(document));
    assert!(
        engine.shadow_root_adopted_style_sheet_tracks_root_for_document_for_test(
            document,
            shadow_root
        )
    );

    let document_source = StyloStylesheetSource::new("body { color: red; }".into(), document_url);
    engine.set_document_adopted_style_sheet_sources(document, vec![document_source.clone()]);
    let after_document_source = engine.computed_cache_generation_for_document_for_test(document);
    engine.set_document_adopted_style_sheet_sources(document, vec![document_source]);
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        after_document_source
    );
    engine.set_document_adopted_style_sheet_sources(document, Vec::new());
    let after_document_clear = engine.computed_cache_generation_for_document_for_test(document);
    assert_eq!(after_document_clear, after_document_source);
    engine.set_document_adopted_style_sheet_sources(document, Vec::new());
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        after_document_clear
    );

    let shadow_source = StyloStylesheetSource::new(":host { color: blue; }".into(), shadow_url);
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source.clone()],
    );
    let after_shadow_source =
        engine.computed_cache_generation_for_document_for_test(active_document);
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![shadow_source],
    );
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(active_document),
        after_shadow_source
    );

    engine.set_owner_style_sheet_text_with_host(&host, owner, ".owner { color: green; }".into());
    let after_owner_source =
        engine.computed_cache_generation_for_document_for_test(active_document);
    engine.set_owner_style_sheet_text_with_host(&host, owner, ".owner { color: green; }".into());
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(active_document),
        after_owner_source
    );

    let linked_source = StyloStylesheetSource::new(".linked { color: black; }".into(), linked_url);
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &url,
        linked_source.clone(),
    );
    let after_linked_source =
        engine.computed_cache_generation_for_document_for_test(active_document);
    engine.record_stylesheet_source_for_url_for_document_for_test(document, &url, linked_source);
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(active_document),
        after_linked_source
    );

    let linked_final_url = url::Url::parse("https://cdn.example.test/linked.css").unwrap();
    let redirected_linked_source =
        StyloStylesheetSource::new(".linked { color: black; }".into(), linked_final_url);
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &url,
        redirected_linked_source.clone(),
    );
    let after_redirected_linked_source =
        engine.computed_cache_generation_for_document_for_test(active_document);
    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &url,
        redirected_linked_source,
    );
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(active_document),
        after_redirected_linked_source
    );
}
#[test]
fn empty_adopted_sources_are_not_retained_but_shadow_roots_remain_trackable() {
    let mut engine = MoliStyleEngine::new();
    let mut host = test_host();
    let document = host.document_handle();
    let shadow_host = host.create_element("section");
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("section should host a shadow root");
    let document_url = url::Url::parse("https://example.test/document.css").unwrap();
    let shadow_url = url::Url::parse("https://example.test/shadow.css").unwrap();
    assert!(host.append_child(document, shadow_host));

    engine.set_document_adopted_style_sheet_sources(
        document,
        vec![StyloStylesheetSource::new(
            "body { color: red; }".into(),
            document_url,
        )],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        shadow_root,
        vec![StyloStylesheetSource::new(
            ":host { color: blue; }".into(),
            shadow_url,
        )],
    );

    assert_eq!(
        engine.adopted_style_sheet_source_owner_counts_for_document_for_test(document),
        (1, 1)
    );
    assert!(engine.document_adopted_style_sheet_tracks_document_for_test(document));
    assert!(
        engine.shadow_root_adopted_style_sheet_tracks_root_for_document_for_test(
            document,
            shadow_root
        )
    );

    engine.set_document_adopted_style_sheet_sources(document, Vec::new());
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(&host, shadow_root, Vec::new());

    assert_eq!(
        engine.adopted_style_sheet_source_owner_counts_for_document_for_test(document),
        (0, 0)
    );
    assert!(engine.document_adopted_style_sheet_tracks_document_for_test(document));
    assert!(
        engine
            .adopted_style_sheet_sources_for_document(document)
            .is_empty()
    );
    assert!(
        engine
            .shadow_root_adopted_style_sheet_sources_with_host(&host, shadow_root)
            .is_empty()
    );
    assert!(
        engine.shadow_root_adopted_style_sheet_tracks_root_for_document_for_test(
            document,
            shadow_root
        )
    );
    let source_scope = StyleSourceScope::for_document_and_connected_shadow_roots(&host, document);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    assert!(
        engine
            .matching_dependency_source_ids_for_document_for_test(
                &host,
                document,
                &source_scope,
                &media
            )
            .is_empty()
    );
}

#[test]
fn inactive_linked_stylesheet_owner_is_not_retained_source_client() {
    let mut host = test_host();
    let document = host.document_handle();
    let link = host.create_element("link");
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "inactive-client.css"));
    assert!(host.append_child(document, link));

    let mut engine = MoliStyleEngine::new();
    let linked_url = url::Url::parse("https://example.test/inactive-client.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        StyloStylesheetSource::new(".linked { color: black; }".into(), linked_url.clone()),
        &[link],
    );

    let linked_source_id =
        StyleSourceId::linked_style_sheet(&host, link).expect("active linked source id");
    assert_eq!(
        engine.retained_stylesheet_source_ids_for_document_for_test(&host, document),
        vec![linked_source_id.clone()]
    );

    let effects = host.set_attribute_effects(link, "disabled", "");
    engine.apply_stylesheet_owner_changes_with_host(&host, effects.stylesheet_owners().changes());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);
    assert!(
        engine
            .retained_stylesheet_source_ids_for_document_for_test(&host, document)
            .is_empty()
    );

    let report = source_lifecycle_report_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
    );
    assert_eq!(
        lifecycle_availability_for(
            report.records(),
            StyleSourceLifecycleOwner::LinkedStyleSheet { owner: link },
        ),
        None
    );
}

#[test]
fn lifecycle_retained_source_gate_ignores_unavailable_tracked_owners() {
    let mut host = test_host();
    let document = host.document_handle();
    let mut engine = MoliStyleEngine::new();

    let disconnected_style = host.create_element("style");
    engine.set_owner_style_sheet_text_with_host(
        &host,
        disconnected_style,
        "body:has(.marker) .target { color: green; }".into(),
    );

    let disconnected_link = host.create_element("link");
    assert!(host.set_attribute(disconnected_link, "rel", "stylesheet"));
    assert!(host.set_attribute(disconnected_link, "href", "disconnected.css"));
    let disconnected_link_url = url::Url::parse("https://example.test/disconnected.css").unwrap();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &disconnected_link_url,
        StyloStylesheetSource::new(
            "body:has(.linked) .target { color: blue; }".into(),
            disconnected_link_url.clone(),
        ),
        &[disconnected_link],
    );

    let inactive_shadow_host = host.create_element("section");
    let inactive_shadow_root = host
        .attach_shadow_root(inactive_shadow_host, "open")
        .expect("inactive host should accept a shadow root");
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        inactive_shadow_root,
        vec![StyloStylesheetSource::new(
            ":host(:has(.marker)) .target { color: purple; }".into(),
            url::Url::parse("https://example.test/inactive-shadow.css").unwrap(),
        )],
    );

    let document_world = engine.world_for_document(document);
    let source_stores = document_world.borrow_source_stores();
    assert!(
        !source_stores.has_document_retained_sources(&host),
        "unavailable document-scope owners must not trigger document retained-source fallback"
    );

    let report = source_stores.source_lifecycle_report(
        &host,
        StyleSourceDocumentContext::for_root_document(document),
    );
    assert_eq!(
        lifecycle_availability_for(
            report.records(),
            StyleSourceLifecycleOwner::OwnerStyleSheet {
                owner: disconnected_style,
            },
        ),
        Some(&StyleSourceLifecycleAvailability::Unavailable {
            reason: StyleSourceLifecycleUnavailableReason::OwnerNotInDocumentTree,
        })
    );
    assert_eq!(
        lifecycle_availability_for(
            report.records(),
            StyleSourceLifecycleOwner::LinkedStyleSheet {
                owner: disconnected_link,
            },
        ),
        Some(&StyleSourceLifecycleAvailability::Unavailable {
            reason: StyleSourceLifecycleUnavailableReason::OwnerNotInDocumentTree,
        })
    );
    assert_eq!(
        lifecycle_availability_for(
            report.records(),
            StyleSourceLifecycleOwner::ShadowRootAdoptedStyleSheets {
                root: inactive_shadow_root,
            },
        ),
        Some(&StyleSourceLifecycleAvailability::Unavailable {
            reason: StyleSourceLifecycleUnavailableReason::InactiveShadowRoot,
        })
    );
}

#[test]
fn style_attribute_impact_classifies_dom_and_stylesheet_inputs() {
    assert!(StyleAttributeImpact::for_attribute_name("style").affects_layout_metric());
    assert!(StyleAttributeImpact::for_attribute_name("style").changes_computed_style());
    assert!(!StyleAttributeImpact::for_attribute_name("style").changes_stylesheet_linkage());

    assert!(StyleAttributeImpact::for_attribute_name("width").affects_layout_metric());
    assert!(!StyleAttributeImpact::for_attribute_name("width").changes_computed_style());
    assert!(!StyleAttributeImpact::for_attribute_name("width").changes_stylesheet_linkage());

    for attribute in ["fill", "stroke", "stroke-width", "paint-order"] {
        assert!(
            StyleAttributeImpact::for_attribute_name(attribute).affects_layout_metric(),
            "SVG presentation attribute {attribute} must invalidate computed paint"
        );
    }

    assert!(StyleAttributeImpact::for_attribute_name("cellpadding").affects_layout_metric());
    assert!(StyleAttributeImpact::for_attribute_name("cellpadding").changes_computed_style());
    assert!(!StyleAttributeImpact::for_attribute_name("cellpadding").changes_stylesheet_linkage());

    assert!(!StyleAttributeImpact::for_attribute_name("href").affects_layout_metric());
    assert!(StyleAttributeImpact::for_attribute_name("href").changes_stylesheet_linkage());

    assert!(StyleAttributeImpact::for_attribute_name("type").affects_layout_metric());
    assert!(StyleAttributeImpact::for_attribute_name("type").changes_stylesheet_linkage());

    assert_eq!(
        StyleAttributeImpact::for_attribute_name("data-state"),
        StyleAttributeImpact::None
    );
}

#[test]
fn attribute_mutation_without_source_dependency_skips_style_invalidation() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let target = host.create_element("div");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(&host, style, ".headline { color: red; }".into());
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert!(
        !engine.test_author_sources_have_attribute_dependency_for_document(
            &host,
            document,
            target,
            "data-state",
        ),
        "the retained source has no dependency on data-state"
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
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
                None
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    assert!(host.set_attribute(target, "data-state", "active"));
    let effects = [StyleMutationEffect::Attribute {
        element: target,
        name: "data-state".to_owned(),
        old_value: None,
        new_value: Some("active".to_owned()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);

    assert_eq!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
        0,
        "attribute changes with no retained source dependency should not fall back to document invalidation"
    );
}

#[test]
fn attribute_mutation_with_source_dependency_still_invalidates_style() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let script = host.create_element("script");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, script));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(
        &host,
        style,
        "script[async] { color: red; }".into(),
    );
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
    assert!(
        engine.test_author_sources_have_attribute_dependency_for_document(
            &host, document, script, "async",
        ),
        "the retained source must report its script async dependency"
    );
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                script,
                "color",
                None,
                &inputs,
                None
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );

    assert!(host.set_attribute(script, "async", ""));
    let effects = [StyleMutationEffect::Attribute {
        element: script,
        name: "async".to_owned(),
        old_value: None,
        new_value: Some(String::new()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);

    assert!(
        engine.pending_style_invalidation_work_item_count_for_document_for_test(document) > 0,
        "attribute changes with retained source dependency must keep invalidating style"
    );
}

#[test]
fn attribute_dependency_change_extracts_class_token_delta() {
    let effect = StyleMutationEffect::Attribute {
        element: NativeNodeId::new(1),
        name: "class".into(),
        old_value: Some("a b a kept".into()),
        new_value: Some("kept c b c".into()),
    };

    let change = effect
        .attribute_dependency_change()
        .expect("attribute mutation should produce dependency change");

    assert_eq!(change.attribute_name, "class");
    assert_eq!(change.removed_class_tokens, vec!["a"]);
    assert_eq!(change.added_class_tokens, vec!["c"]);
    assert_eq!(change.removed_id, None);
    assert_eq!(change.added_id, None);
}
#[test]
fn attribute_dependency_change_requires_pre_normalized_class_and_id_names() {
    let class_effect = StyleMutationEffect::Attribute {
        element: NativeNodeId::new(1),
        name: "CLASS".into(),
        old_value: Some("old".into()),
        new_value: Some("new".into()),
    };
    let id_effect = StyleMutationEffect::Attribute {
        element: NativeNodeId::new(1),
        name: "ID".into(),
        old_value: Some("old".into()),
        new_value: Some("new".into()),
    };

    let class_change = class_effect
        .attribute_dependency_change()
        .expect("attribute mutation should produce dependency change");
    let id_change = id_effect
        .attribute_dependency_change()
        .expect("attribute mutation should produce dependency change");

    assert_eq!(class_change.attribute_name, "CLASS");
    assert!(class_change.removed_class_tokens.is_empty());
    assert!(class_change.added_class_tokens.is_empty());
    assert_eq!(id_change.attribute_name, "ID");
    assert_eq!(id_change.removed_id, None);
    assert_eq!(id_change.added_id, None);
    assert!(!stylo_attribute_change_can_skip_fallback_without_dependency("DATA-State"));
}
#[test]
fn direct_namespaced_attribute_style_effect_preserves_case_on_html_elements() {
    let mut host = test_host();
    let element = host.create_element("div");

    let effect = StyleMutationEffect::attribute_for_element_ns(
        &host,
        element,
        Some("urn:test"),
        "DATA-State",
        None,
        Some("marker".into()),
    );

    assert!(matches!(
        effect,
        StyleMutationEffect::Attribute {
            element: target,
            name,
            old_value: None,
            new_value: Some(value),
        } if target == element && name == "DATA-State" && value == "marker"
    ));
}
#[test]
fn removed_element_dependency_snapshot_dedupes_class_tokens() {
    let mut host = test_host();
    let element = host.create_element("div");
    assert!(host.set_attribute(element, "class", "a b a c b"));

    let snapshot =
        style_element_dependency_snapshot(&host, element).expect("element should have snapshot");

    assert_eq!(snapshot.class_tokens(), ["a", "b", "c"]);
}
#[test]
fn attribute_dependency_change_keeps_id_delta_and_attribute_probe() {
    let id_effect = StyleMutationEffect::Attribute {
        element: NativeNodeId::new(1),
        name: "id".into(),
        old_value: Some("old".into()),
        new_value: Some("new".into()),
    };
    let data_effect = StyleMutationEffect::Attribute {
        element: NativeNodeId::new(1),
        name: "data-state".into(),
        old_value: Some("inactive".into()),
        new_value: Some("active".into()),
    };

    let id_change = id_effect
        .attribute_dependency_change()
        .expect("id mutation should produce dependency change");
    assert_eq!(id_change.removed_id.as_deref(), Some("old"));
    assert_eq!(id_change.added_id.as_deref(), Some("new"));

    let data_change = data_effect
        .attribute_dependency_change()
        .expect("ordinary attribute mutation should produce dependency change");
    assert_eq!(data_change.attribute_name, "data-state");
    assert!(data_change.removed_class_tokens.is_empty());
    assert!(data_change.added_class_tokens.is_empty());
    assert_eq!(data_change.removed_id, None);
    assert_eq!(data_change.added_id, None);
}
#[test]
fn typed_attribute_style_effect_captures_exact_new_value_without_observer_records() {
    let mut host = test_host();
    let element = host.create_element("div");
    assert!(host.append_child(host.document_handle(), element));
    let effects = host.set_attribute_effects(element, "class", "marker target");
    assert!(effects.observer_records().records().is_empty());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);

    assert!(style_effects.iter().any(|effect| matches!(
        effect,
        StyleMutationEffect::Attribute {
            element: target,
            name,
            old_value,
            new_value,
        } if *target == element
            && name == "class"
            && old_value.is_none()
            && new_value.as_deref() == Some("marker target")
    )));
    let change = style_effects
        .iter()
        .find_map(StyleMutationEffect::attribute_dependency_change)
        .expect("attribute style effect should expose dependency change");
    assert_eq!(change.added_class_tokens, ["marker", "target"]);
}
#[test]
fn typed_attribute_style_effect_normalizes_html_attribute_name() {
    let mut host = test_host();
    let element = host.create_element("div");
    assert!(host.append_child(host.document_handle(), element));
    let effects = host.set_attribute_effects(element, "CLASS", "marker");
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);

    assert!(style_effects.iter().any(|effect| matches!(
        effect,
        StyleMutationEffect::Attribute {
            element: target,
            name,
            old_value,
            new_value,
        } if *target == element
            && name == "class"
            && old_value.is_none()
            && new_value.as_deref() == Some("marker")
    )));
}
#[test]
fn typed_attribute_style_effect_preserves_namespaced_attribute_name() {
    let mut host = test_host();
    let element = host.create_element("div");
    assert!(host.append_child(host.document_handle(), element));
    let effects = host.set_attribute_ns_effects(
        element,
        Some("urn:test"),
        Some("test"),
        "DATA-State",
        "test:DATA-State",
        "marker",
    );
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&host, &effects);

    assert!(style_effects.iter().any(|effect| matches!(
        effect,
        StyleMutationEffect::Attribute {
            element: target,
            name,
            old_value,
            new_value,
        } if *target == element
            && name == "DATA-State"
            && old_value.is_none()
            && new_value.as_deref() == Some("marker")
    )));
}
