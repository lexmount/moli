use super::*;

#[test]
fn viewport_resize_recascades_cached_viewport_units_without_media_match_changes() {
    reset_source_cascade_rebuild_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let rule_target = host.create_element("div");
    assert!(host.set_attribute(rule_target, "class", "rule-target stable-media-target"));
    assert!(host.append_child(document, rule_target));
    let inline_target = host.create_element("div");
    assert!(host.set_attribute(inline_target, "style", "width: 12vw; height: 25vh",));
    assert!(host.append_child(document, inline_target));
    let shadow_host = host.create_element("section");
    assert!(host.append_child(document, shadow_host));
    let shadow_root = host
        .attach_shadow_root(shadow_host, "open")
        .expect("the viewport-unit host should accept a shadow root");
    let shadow_target = host.create_element("span");
    assert!(host.set_attribute(shadow_target, "class", "shadow-target"));
    assert!(host.append_child(shadow_root, shadow_target));

    let document_url = url::Url::parse("https://example.test/viewport-units.html").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            r#"
              .rule-target {
                position: absolute;
                width: 10vw;
                height: 10vh;
                min-width: 2vmin;
                max-width: 2vmax;
                font-size: 2vmin;
                padding-left: calc(5vw + 8px);
                top: 10dvh;
                right: 10svh;
                bottom: 10lvh;
              }
              @media (min-width: 1px) {
                .stable-media-target { left: 50vw; }
              }
            "#
            .into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 81,
        ))),
    );
    inputs.shadow_stylesheet_sources.push((
        shadow_root,
        vec![
            StyloStylesheetSource::new(
                ".shadow-target { width: 25vw; height: 25vh; }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(StyleSourceId::shadow_root_adopted_style_sheet(
                shadow_root,
                82,
            ))),
        ],
    ));

    let engine = MoliStyleEngine::new();
    let viewport_1000 = StyleViewport::new(Some(1000.0), Some(800.0));
    let read = |target, property, viewport| {
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
                property,
                None,
                &inputs,
                viewport,
            )
            .unwrap_or_else(|| panic!("{property} should have a computed value"))
    };
    let rule_properties = [
        "width",
        "height",
        "min-width",
        "max-width",
        "font-size",
        "padding-left",
        "top",
        "right",
        "bottom",
        "left",
    ];
    assert_eq!(
        rule_properties.map(|property| read(rule_target, property, viewport_1000)),
        [
            "100px", "80px", "16px", "20px", "16px", "58px", "80px", "80px", "80px", "500px",
        ],
    );
    assert_eq!(read(inline_target, "width", viewport_1000), "120px");
    assert_eq!(read(inline_target, "height", viewport_1000), "200px");
    assert_eq!(read(shadow_target, "width", viewport_1000), "250px");
    assert_eq!(read(shadow_target, "height", viewport_1000), "200px");
    let style_before = retained_primary_style_for_test(&engine, &host, rule_target)
        .expect("the initial viewport-relative style should be retained");
    let document_flushes = engine.retained_stylist_flush_count_for_document_for_test(document);
    let shadow_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, shadow_root)
        .expect("the shadow scope should be retained");
    let source_rebuilds = source_cascade_rebuild_count_for_test();

    let viewport_500 = StyleViewport::new(Some(500.0), Some(400.0));
    assert_eq!(
        rule_properties.map(|property| read(rule_target, property, viewport_500)),
        [
            "50px", "40px", "8px", "10px", "8px", "33px", "40px", "40px", "40px", "250px",
        ],
        "the same cached element must recascade every viewport-relative unit",
    );
    assert_eq!(read(inline_target, "width", viewport_500), "60px");
    assert_eq!(read(inline_target, "height", viewport_500), "100px");
    assert_eq!(read(shadow_target, "width", viewport_500), "125px");
    assert_eq!(read(shadow_target, "height", viewport_500), "100px");
    let style_after = retained_primary_style_for_test(&engine, &host, rule_target)
        .expect("the resized viewport-relative style should be retained");
    assert!(
        !ServoArc::ptr_eq(&style_before, &style_after),
        "viewport invalidation must replace the canonical ComputedValues on the same element",
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        document_flushes,
        "an always-matching media query must not be used to hide viewport-unit invalidation",
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, shadow_root),
        Some(shadow_flushes),
        "viewport units alone must not flush the ShadowRoot AuthorStyles",
    );
    assert_eq!(
        source_cascade_rebuild_count_for_test(),
        source_rebuilds,
        "viewport units alone must not rebuild source-local cascade data",
    );
}

#[test]
fn repeated_viewport_resizes_recascade_only_dependent_cached_elements() {
    let mut host = test_host();
    let document = host.document_handle();
    let dependent = host.create_element("div");
    let independent = host.create_element("div");
    assert!(host.set_attribute(dependent, "class", "viewport-dependent"));
    assert!(host.set_attribute(independent, "class", "viewport-independent"));
    assert!(host.append_child(document, dependent));
    assert!(host.append_child(document, independent));

    let document_url = url::Url::parse("https://example.test/repeated-resize.html").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            r#"
              .viewport-dependent { width: 10vw; height: 10vh; }
              .viewport-independent { width: 123px; height: 45px; }
            "#
            .into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 83,
        ))),
    );

    let engine = MoliStyleEngine::new();
    let read = |target, property, viewport| {
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
                property,
                None,
                &inputs,
                viewport,
            )
            .unwrap_or_else(|| panic!("{property} should have a computed value"))
    };
    let large = StyleViewport::new(Some(800.0), Some(600.0));
    assert_eq!(read(dependent, "width", large), "80px");
    assert_eq!(read(independent, "width", large), "123px");
    let first_dependent = retained_primary_style_for_test(&engine, &host, dependent).unwrap();
    let first_independent = retained_primary_style_for_test(&engine, &host, independent).unwrap();
    let flushes = engine.retained_stylist_flush_count_for_document_for_test(document);

    let small = StyleViewport::new(Some(400.0), Some(300.0));
    assert_eq!(read(dependent, "width", small), "40px");
    assert_eq!(read(dependent, "height", small), "30px");
    assert_eq!(read(independent, "width", small), "123px");
    let second_dependent = retained_primary_style_for_test(&engine, &host, dependent).unwrap();
    let second_independent = retained_primary_style_for_test(&engine, &host, independent).unwrap();
    assert!(!ServoArc::ptr_eq(&first_dependent, &second_dependent));
    assert!(
        ServoArc::ptr_eq(&first_independent, &second_independent),
        "a viewport-independent cached element must survive a resize unchanged",
    );

    assert_eq!(read(dependent, "width", large), "80px");
    assert_eq!(read(dependent, "height", large), "60px");
    assert_eq!(read(independent, "height", large), "45px");
    let third_dependent = retained_primary_style_for_test(&engine, &host, dependent).unwrap();
    let third_independent = retained_primary_style_for_test(&engine, &host, independent).unwrap();
    assert!(!ServoArc::ptr_eq(&second_dependent, &third_dependent));
    assert!(
        ServoArc::ptr_eq(&second_independent, &third_independent),
        "reversing a resize must still preserve viewport-independent ComputedValues",
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        flushes,
        "viewport-unit recascade must not flush an unchanged stylesheet",
    );
}

#[test]
fn viewport_resize_propagates_through_inheritance_variables_pseudos_and_nested_shadow_roots() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let child = host.create_element("span");
    let pseudo_target = host.create_element("div");
    assert!(host.set_attribute(parent, "class", "viewport-parent"));
    assert!(host.set_attribute(child, "class", "viewport-child"));
    assert!(host.set_attribute(pseudo_target, "class", "viewport-pseudo"));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, child));
    assert!(host.append_child(document, pseudo_target));

    let outer_host = host.create_element("article");
    assert!(host.append_child(document, outer_host));
    let outer_root = host
        .attach_shadow_root(outer_host, "open")
        .expect("outer host should accept a shadow root");
    let outer_target = host.create_element("div");
    let inner_host = host.create_element("aside");
    assert!(host.set_attribute(outer_target, "class", "outer-viewport-target"));
    assert!(host.append_child(outer_root, outer_target));
    assert!(host.append_child(outer_root, inner_host));
    let inner_root = host
        .attach_shadow_root(inner_host, "open")
        .expect("a host inside a ShadowRoot should accept a nested shadow root");
    let inner_target = host.create_element("span");
    assert!(host.set_attribute(inner_target, "class", "inner-viewport-target"));
    assert!(host.append_child(inner_root, inner_target));

    let document_url = url::Url::parse("https://example.test/viewport-dependencies.html").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            r#"
              .viewport-parent { font-size: 2vmin; --viewport-gap: 10vw; }
              .viewport-child { font-size: 2em; padding-left: var(--viewport-gap); }
              .viewport-pseudo::before {
                content: "viewport";
                width: 10vw;
                height: 10vh;
              }
            "#
            .into(),
            document_url.clone(),
        )
        .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
            document, 84,
        ))),
    );
    inputs.shadow_stylesheet_sources.push((
        outer_root,
        vec![
            StyloStylesheetSource::new(
                ".outer-viewport-target { width: 20vw; }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(StyleSourceId::shadow_root_adopted_style_sheet(
                outer_root, 85,
            ))),
        ],
    ));
    inputs.shadow_stylesheet_sources.push((
        inner_root,
        vec![
            StyloStylesheetSource::new(
                ".inner-viewport-target { height: 15vh; }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(StyleSourceId::shadow_root_adopted_style_sheet(
                inner_root, 86,
            ))),
        ],
    ));

    let engine = MoliStyleEngine::new();
    let read = |target, property, pseudo, viewport| {
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
                property,
                pseudo,
                &inputs,
                viewport,
            )
            .unwrap_or_else(|| panic!("{property} should have a computed value"))
    };
    let large = StyleViewport::new(Some(1000.0), Some(800.0));
    assert_eq!(read(parent, "font-size", None, large), "16px");
    assert_eq!(read(child, "font-size", None, large), "32px");
    assert_eq!(read(child, "padding-left", None, large), "100px");
    assert_eq!(read(pseudo_target, "width", Some("before"), large), "100px");
    assert_eq!(read(pseudo_target, "height", Some("before"), large), "80px");
    assert_eq!(read(outer_target, "width", None, large), "200px");
    assert_eq!(read(inner_target, "height", None, large), "120px");
    let child_before = retained_primary_style_for_test(&engine, &host, child).unwrap();
    let nested_before = retained_primary_style_for_test(&engine, &host, inner_target).unwrap();
    let outer_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, outer_root)
        .expect("outer shadow scope should be retained");
    let inner_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, inner_root)
        .expect("inner shadow scope should be retained");

    let small = StyleViewport::new(Some(500.0), Some(400.0));
    assert_eq!(read(parent, "font-size", None, small), "8px");
    assert_eq!(
        read(child, "font-size", None, small),
        "16px",
        "viewport invalidation must propagate through inherited font metrics",
    );
    assert_eq!(
        read(child, "padding-left", None, small),
        "50px",
        "a viewport unit substituted through an inherited custom property must recascade",
    );
    assert_eq!(read(pseudo_target, "width", Some("before"), small), "50px");
    assert_eq!(read(pseudo_target, "height", Some("before"), small), "40px");
    assert_eq!(read(outer_target, "width", None, small), "100px");
    assert_eq!(read(inner_target, "height", None, small), "60px");
    let child_after = retained_primary_style_for_test(&engine, &host, child).unwrap();
    let nested_after = retained_primary_style_for_test(&engine, &host, inner_target).unwrap();
    assert!(!ServoArc::ptr_eq(&child_before, &child_after));
    assert!(!ServoArc::ptr_eq(&nested_before, &nested_after));
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, outer_root),
        Some(outer_flushes),
        "viewport dependency invalidation must not rebuild the outer AuthorStyles",
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, inner_root),
        Some(inner_flushes),
        "viewport dependency invalidation must not rebuild nested AuthorStyles",
    );
}

#[test]
fn device_changes_keep_media_sheets_installed_and_flush_only_affected_tree_scopes() {
    reset_source_cascade_rebuild_count_for_test();
    let mut host = test_host();
    let document = host.document_handle();
    let document_target = host.create_element("div");
    assert!(host.set_attribute(document_target, "class", "document-target"));
    assert!(host.set_attribute(document_target, "style", "width: 50vw"));
    assert!(host.append_child(document, document_target));
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
    let first_target = host.create_element("span");
    let second_target = host.create_element("span");
    assert!(host.set_attribute(first_target, "class", "first-target"));
    assert!(host.set_attribute(first_target, "style", "height: 50vh"));
    assert!(host.set_attribute(second_target, "class", "second-target"));
    assert!(host.append_child(first_root, first_target));
    assert!(host.append_child(second_root, second_target));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/shadow-media.html").unwrap();
    let first_source_id = StyleSourceId::shadow_root_adopted_style_sheet(first_root, 71);
    let second_source_id = StyleSourceId::shadow_root_adopted_style_sheet(second_root, 72);
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs.document_stylesheet_sources.push(
        StyloStylesheetSource::new(
            ".document-target { color: rgb(7, 8, 9); }".into(),
            document_url.clone(),
        )
        .with_owner_media_text("(max-width: 600px)"),
    );
    inputs.shadow_stylesheet_sources.push((
        first_root,
        vec![
            StyloStylesheetSource::new(
                ".first-target { color: rgb(1, 2, 3); }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(first_source_id.clone()))
            .with_owner_media_text("(max-width: 600px)"),
        ],
    ));
    inputs.shadow_stylesheet_sources.push((
        second_root,
        vec![
            StyloStylesheetSource::new(
                ".second-target { color: rgb(4, 5, 6); }".into(),
                document_url.clone(),
            )
            .with_source_id(Some(second_source_id.clone()))
            .with_owner_media_text("(min-width: 100px)"),
        ],
    ));

    let viewport_800 = StyleViewport::new(Some(800.0), Some(600.0));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            document_target,
            "color",
            None,
            &inputs,
            viewport_800,
        ),
        Some("rgb(0, 0, 0)".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "color",
            None,
            &inputs,
            viewport_800,
        ),
        Some("rgb(0, 0, 0)".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            second_target,
            "color",
            None,
            &inputs,
            viewport_800,
        ),
        Some("rgb(4, 5, 6)".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            document_target,
            "width",
            None,
            &inputs,
            viewport_800,
        ),
        Some("400px".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "height",
            None,
            &inputs,
            viewport_800,
        ),
        Some("300px".into()),
    );
    engine.with_retained_style_system_for_document_for_test(document, |retained| {
        assert_eq!(
            retained.document_stylesheets.entries().len(),
            1,
            "a non-matching owner MediaList must not remove its sheet from the Document scope",
        );
        assert_eq!(retained.shadow_scopes.len(), 2);
        assert!(
            retained
                .shadow_scopes
                .iter()
                .all(|scope| scope.active_stylesheets().entries().len() == 1),
            "a non-matching owner MediaList must not remove its sheet from the TreeScope",
        );
    });
    materialize_source_cascade_data_for_document_for_test(
        &engine,
        &host,
        document,
        StyleSourceDocumentContext::for_root_document(document),
        [first_source_id, second_source_id],
    );
    let document_flushes = engine.retained_stylist_flush_count_for_document_for_test(document);
    let first_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, first_root)
        .expect("first scope should be retained");
    let second_flushes = engine
        .retained_shadow_scope_flush_count_for_document_for_test(document, second_root)
        .expect("second scope should be retained");
    let source_rebuilds = source_cascade_rebuild_count_for_test();

    let viewport_700 = StyleViewport::new(Some(700.0), Some(500.0));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "color",
            None,
            &inputs,
            viewport_700,
        ),
        Some("rgb(0, 0, 0)".into()),
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, first_root),
        Some(first_flushes),
        "a viewport change that crosses no media boundary must not flush the first scope",
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, second_root),
        Some(second_flushes),
        "a viewport change that crosses no media boundary must not flush the second scope",
    );
    assert_eq!(source_cascade_rebuild_count_for_test(), source_rebuilds);
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            document_target,
            "width",
            None,
            &inputs,
            viewport_700,
        ),
        Some("350px".into()),
        "viewport units must be invalidated without a Document stylesheet flush",
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "height",
            None,
            &inputs,
            viewport_700,
        ),
        Some("250px".into()),
        "viewport units inside ShadowRoot must be invalidated without an AuthorStyles flush",
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        document_flushes,
        "a viewport change that crosses no media boundary must not flush the Document Stylist",
    );

    let viewport_500 = StyleViewport::new(Some(500.0), Some(600.0));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            document_target,
            "color",
            None,
            &inputs,
            viewport_500,
        ),
        Some("rgb(7, 8, 9)".into()),
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            first_target,
            "color",
            None,
            &inputs,
            viewport_500,
        ),
        Some("rgb(1, 2, 3)".into()),
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, first_root),
        Some(first_flushes + 1),
        "only the scope whose top-level MediaList changed must flush",
    );
    assert_eq!(
        engine.retained_shadow_scope_flush_count_for_document_for_test(document, second_root),
        Some(second_flushes),
        "an unrelated matching MediaList must preserve its AuthorStyles",
    );
    assert_eq!(
        engine.retained_stylist_flush_count_for_document_for_test(document),
        document_flushes + 1,
        "the Document Stylist must flush once when its installed MediaList changes match state",
    );
    assert_eq!(
        source_cascade_rebuild_count_for_test(),
        source_rebuilds + 1,
        "only the affected scope's source-local cascade projection must rebuild",
    );
}

#[test]
fn style_subtree_invalidation_clears_only_affected_shadow_cascade_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_shadow_host = host.create_element("section");
    let second_shadow_host = host.create_element("article");
    assert!(host.append_child(document, first_shadow_host));
    assert!(host.append_child(document, second_shadow_host));
    let first_shadow_root = host
        .attach_shadow_root(first_shadow_host, "open")
        .expect("first host should accept a shadow root");
    let second_shadow_root = host
        .attach_shadow_root(second_shadow_host, "open")
        .expect("second host should accept a shadow root");

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        ":host { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let second_source = StyloStylesheetSource::new(
        ":host { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        first_shadow_root,
        vec![first_source.clone()],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        second_shadow_root,
        vec![second_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((first_shadow_root, vec![first_source]));
    inputs
        .shadow_stylesheet_sources
        .push((second_shadow_root, vec![second_source]));
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);

    let (first_cascade_data, second_cascade_data) = engine
        .with_retained_style_system_for_document_for_test(document, |retained| {
            let first = retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == first_shadow_root)
                .expect("retained system should track the first shadow root")
                .1
                .clone();
            let second = retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == second_shadow_root)
                .expect("retained system should track the second shadow root")
                .1
                .clone();
            (first, second)
        });
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            first_shadow_root,
            first_cascade_data,
        );
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            second_shadow_root,
            second_cascade_data,
        );

    engine.invalidate_style_subtree(&host, first_shadow_host);

    assert!(
        !engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(first_shadow_root)
    );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(second_shadow_root)
    );
}

#[test]
fn detached_subtree_invalidation_clears_only_affected_shadow_cascade_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let first_shadow_host = host.create_element("section");
    let second_shadow_host = host.create_element("article");
    assert!(host.append_child(document, first_shadow_host));
    assert!(host.append_child(document, second_shadow_host));
    let first_shadow_root = host
        .attach_shadow_root(first_shadow_host, "open")
        .expect("first host should accept a shadow root");
    let second_shadow_root = host
        .attach_shadow_root(second_shadow_host, "open")
        .expect("second host should accept a shadow root");

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        ":host { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let second_source = StyloStylesheetSource::new(
        ":host { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        first_shadow_root,
        vec![first_source.clone()],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        second_shadow_root,
        vec![second_source.clone()],
    );
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .shadow_stylesheet_sources
        .push((first_shadow_root, vec![first_source]));
    inputs
        .shadow_stylesheet_sources
        .push((second_shadow_root, vec![second_source]));
    let key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);

    let (first_cascade_data, second_cascade_data) = engine
        .with_retained_style_system_for_document_for_test(document, |retained| {
            let first = retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == first_shadow_root)
                .expect("retained system should track the first shadow root")
                .1
                .clone();
            let second = retained
                .shadow_cascade_data
                .iter()
                .find(|(root, _)| *root == second_shadow_root)
                .expect("retained system should track the second shadow root")
                .1
                .clone();
            (first, second)
        });
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            first_shadow_root,
            first_cascade_data,
        );
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            second_shadow_root,
            second_cascade_data,
        );

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(
        &host,
        &[StyleMutationEffect::DisconnectedSubtrees {
            roots: vec![first_shadow_host].into(),
        }],
        &media,
    );

    assert!(
        !engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(first_shadow_root)
    );
    assert!(
        engine
            .dom_adapter
            .has_shadow_cascade_data_for_test(second_shadow_root)
    );
}

#[test]
fn document_stylesheet_dirty_mark_isolated_to_its_document_world() {
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
    let active_source = StyloStylesheetSource::new(
        ":host { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let detached_source = StyloStylesheetSource::new(
        ":host { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        active_shadow_root,
        vec![active_source.clone()],
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        detached_shadow_root,
        vec![detached_source.clone()],
    );

    let mut active_inputs = FullStyleWorldSnapshot::default();
    active_inputs
        .shadow_stylesheet_sources
        .push((active_shadow_root, vec![active_source]));
    let active_key = StyleWorldKey::new(&active_inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, active_key, &active_inputs);

    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .shadow_stylesheet_sources
        .push((detached_shadow_root, vec![detached_source]));
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
    ensure_adapter_element_data(&engine, &host, active_shadow_host);
    ensure_adapter_element_data(&engine, &host, detached_shadow_host);
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            document,
            active_shadow_root,
            active_cascade_data,
        );
    engine
        .dom_adapter
        .set_shadow_cascade_data_for_document_for_test(
            detached_document,
            detached_shadow_root,
            detached_cascade_data,
        );
    assert!(engine.dom_adapter.has_element_data(active_shadow_host));
    assert!(engine.dom_adapter.has_element_data(detached_shadow_host));
    assert_eq!(
        engine.dom_adapter.shadow_cascade_document_count_for_test(),
        2
    );

    engine.mark_document_stylesheet_set_dirty(detached_document);

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
    assert!(engine.dom_adapter.has_element_data(active_shadow_host));
    assert!(engine.dom_adapter.has_element_data(detached_shadow_host));
    assert_eq!(
        engine.dom_adapter.shadow_cascade_document_count_for_test(),
        2
    );
    assert_only_source_document_is_dirty(&engine, detached_document, document);
}

#[test]
fn document_replacement_cleanup_preserves_unreplaced_document_world_and_shared_sources() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let active_link = host.create_element("link");
    assert!(host.append_child(document, active));
    assert!(host.append_child(document, active_link));

    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    let detached_link = host.create_element("link");
    assert!(host.append_child(detached_document, detached));
    assert!(host.append_child(detached_document, detached_link));

    for link in [active_link, detached_link] {
        assert!(host.set_attribute(link, "rel", "stylesheet"));
        assert!(host.set_attribute(link, "href", "shared.css"));
    }

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let linked_url = url::Url::parse("https://example.test/shared.css").unwrap();
    let linked_source = StyloStylesheetSource::new(
        "main, section { color: rgb(1, 2, 3); }".into(),
        linked_url.clone(),
    );
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &linked_url,
        linked_source,
        &[active_link, detached_link],
    );

    let inputs = FullStyleWorldSnapshot::default();
    for handle in [active, detached] {
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
        ensure_adapter_element_data(&engine, &host, handle);
    }
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1)
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(detached_document),
        (1, 1)
    );
    assert!(engine.dom_adapter.has_element_data(active));
    assert!(engine.dom_adapter.has_element_data(detached));
    let document_source_set_generation =
        engine.source_set_generation_for_document_for_test(document);
    let detached_source_set_generation =
        engine.source_set_generation_for_document_for_test(detached_document);
    let detached_generation =
        engine.computed_cache_generation_for_document_for_test(detached_document);

    engine.clear_for_document_replacement(document);

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (0, 0)
    );
    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(detached_document),
        (1, 1)
    );
    assert!(!engine.dom_adapter.has_element_data(active));
    assert!(engine.dom_adapter.has_element_data(detached));
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(detached_document),
        detached_generation
    );
    assert!(
        engine.source_set_generation_for_document_for_test(document)
            > document_source_set_generation
    );
    assert_eq!(
        engine.source_set_generation_for_document_for_test(detached_document),
        detached_source_set_generation
    );
    assert_eq!(
        engine.stylesheet_text_for_url_for_document_for_test(document, &linked_url),
        None
    );
    assert_eq!(
        engine.stylesheet_text_for_url_for_document_for_test(detached_document, &linked_url),
        Some("main, section { color: rgb(1, 2, 3); }".into())
    );
}

#[test]
fn retired_document_world_releases_heavy_state_without_losing_its_generation_barrier() {
    let mut host = test_host();
    let document = host.create_detached_html_document();
    let target = host.create_element("div");
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                target,
                "display",
                None,
                &inputs,
                None,
            )
            .is_some()
    );
    assert!(engine.dom_adapter.has_element_data(target));
    assert!(engine.document_style_world_is_active_for_test(document));
    assert_eq!(engine.active_document_style_world_count_for_test(), 1);
    let weak_world = std::rc::Rc::downgrade(&engine.world_for_document(document));
    let published_generation = engine.computed_cache_generation_for_document_for_test(document);

    assert!(engine.retire_document_style_world(document));
    assert!(
        weak_world.upgrade().is_none(),
        "retirement must release the world allocation, not merely remove its contents"
    );
    assert!(!engine.document_style_world_is_active_for_test(document));
    assert_eq!(engine.active_document_style_world_count_for_test(), 0);
    assert!(!engine.dom_adapter.has_element_data(target));

    let retired_generation = engine.computed_cache_generation_for_document_for_test(document);
    assert!(
        retired_generation > published_generation,
        "retirement must invalidate wrappers published by the old world"
    );
    assert_eq!(
        engine.active_document_style_world_count_for_test(),
        0,
        "a stale wrapper generation read must not resurrect the retired world"
    );
    assert!(
        engine
            .stylesheet_resource_snapshot_for_document(document)
            .is_none()
    );
    assert!(
        engine
            .retained_stylesheet_query_snapshot_for_document(document)
            .is_none()
    );
    assert_eq!(engine.active_document_style_world_count_for_test(), 0);

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        retired_generation
    );
    assert!(!engine.document_style_world_is_active_for_test(document));
}

#[test]
fn detached_retained_rebuild_preserves_active_document_adapter_element_data() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    let active_key = StyleWorldKey::new(&inputs, None);
    engine.ensure_retained_style_system_for_document(&host, document, active_key, &inputs);
    ensure_adapter_element_data(&engine, &host, active);
    assert!(engine.dom_adapter.has_element_data(active));

    let detached_source = StyloStylesheetSource::new(
        "section { color: rgb(1, 2, 3); }".into(),
        document_url.clone(),
    );
    let mut first_detached_inputs = FullStyleWorldSnapshot::default();
    first_detached_inputs
        .document_stylesheet_sources
        .push(detached_source);
    let first_detached_key = StyleWorldKey::new(&first_detached_inputs, None);
    engine.ensure_retained_style_system_for_document(
        &host,
        detached_document,
        first_detached_key,
        &first_detached_inputs,
    );

    let next_detached_source = StyloStylesheetSource::new(
        "section { color: rgb(4, 5, 6); }".into(),
        document_url.clone(),
    );
    let mut next_detached_inputs = FullStyleWorldSnapshot::default();
    next_detached_inputs
        .document_stylesheet_sources
        .push(next_detached_source);
    let next_detached_key = StyleWorldKey::new(&next_detached_inputs, None);
    engine.ensure_retained_style_system_for_document(
        &host,
        detached_document,
        next_detached_key,
        &next_detached_inputs,
    );

    assert!(
        engine.dom_adapter.has_element_data(active),
        "rebuilding detached document retained style system must not clear active document adapter data"
    );
}
