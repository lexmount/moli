use super::*;

#[test]
fn renderer_uses_the_published_cssom_block_only_in_its_parse_context() {
    use crate::css_style::{
        CssInlineStyleDeclarationState, CssomStyleAttribute, InlineStyleParseContext,
    };
    use style::{context::QuirksMode, stylesheets::UrlExtraData};

    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    let source_text = "width: 10px;";
    assert!(host.append_child(document, target));
    assert!(host.set_attribute(target, "style", source_text));
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut block = moli_css_parse::parse_declaration_block(source_text);
    let published = block.publish();
    let mut engine = MoliStyleEngine::new();
    host.replace_style_attribute_source(
        target,
        Box::new(CssomStyleAttribute::with_text(
            CssInlineStyleDeclarationState {
                block,
                parse_context: Some(InlineStyleParseContext {
                    document,
                    url_data: UrlExtraData::from(document_url.clone()),
                    quirks_mode: QuirksMode::NoQuirks,
                }),
                ..Default::default()
            },
            source_text.to_owned(),
            None,
            true,
        )),
    );
    let inputs = FullStyleWorldSnapshot::default();
    let read = |engine: &MoliStyleEngine| {
        assert_eq!(
            engine
                .computed_style_property_value(
                    &host,
                    &document_url,
                    target,
                    "width",
                    None,
                    &inputs,
                    None,
                )
                .as_deref(),
            Some("10px")
        );
        engine.dom_adapter.with_bound_host(&host, |binding| {
            binding
                .element(&host, target)
                .unwrap()
                .style_attribute()
                .unwrap()
                .clone_arc()
        })
    };
    assert!(ServoArc::ptr_eq(&published, &read(&engine)));

    engine.set_inline_style_base_url_with_host(
        &host,
        target,
        url::Url::parse("https://example.test/changed/").unwrap(),
    );
    engine.invalidate_style_subtree(&host, target);
    assert!(!ServoArc::ptr_eq(&published, &read(&engine)));
}

#[test]
fn retained_inline_style_refreshes_only_when_its_parsing_input_changes() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("section");
    let target = host.create_element("span");
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, target));
    assert!(host.set_attribute(target, "style", "background-image: url(icon.png)"));
    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let inputs = FullStyleWorldSnapshot::default();
    let read = |engine: &MoliStyleEngine, host: &DomHost| {
        engine.computed_style_property_value(
            host,
            &document_url,
            target,
            "background-image",
            None,
            &inputs,
            None,
        )
    };
    let snapshot = |engine: &MoliStyleEngine, host: &DomHost| {
        engine.dom_adapter.with_bound_host(host, |binding| {
            binding
                .element(host, target)?
                .style_attribute()
                .map(|value| value.clone_arc())
        })
    };

    assert_eq!(
        read(&engine, &host).as_deref(),
        Some("url(\"https://example.test/icon.png\")")
    );
    let initial = snapshot(&engine, &host).unwrap();
    engine.invalidate_style_subtree(&host, parent);
    read(&engine, &host).unwrap();
    assert!(ServoArc::ptr_eq(
        &initial,
        &snapshot(&engine, &host).unwrap()
    ));

    let new_base = url::Url::parse("https://example.test/new/").unwrap();
    engine.set_inline_style_base_url_with_host(&host, target, new_base);
    engine.invalidate_style_subtree(&host, parent);
    assert_eq!(
        read(&engine, &host).as_deref(),
        Some("url(\"https://example.test/new/icon.png\")")
    );
    assert!(!ServoArc::ptr_eq(
        &initial,
        &snapshot(&engine, &host).unwrap()
    ));

    assert!(host.remove_attribute(target, "style"));
    engine.invalidate_style_subtree(&host, parent);
    assert_eq!(read(&engine, &host).as_deref(), Some("none"));
    assert!(snapshot(&engine, &host).is_none());
}

#[test]
fn detached_document_inline_style_metadata_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached = host.create_element("section");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let detached_base_url = url::Url::parse("https://detached.test/cssom/").unwrap();
    engine.set_inline_style_base_url_with_host(&host, detached, detached_base_url.clone());
    set_test_inline_render_text(&mut host, detached, "background-image: url(icon.png);");

    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(detached_document),
        1
    );
    assert_eq!(
        engine.inline_style_base_url_with_host(&host, detached),
        Some(detached_base_url)
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            detached,
            "background-image",
            None,
            &FullStyleWorldSnapshot::default(),
            None,
        ),
        Some("url(\"https://detached.test/cssom/icon.png\")".into())
    );

    engine.clear_inline_style_base_url_with_host(&host, detached);

    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(detached_document),
        0
    );
}

#[test]
fn inline_style_metadata_moves_to_current_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("section");
    let detached_document = host.create_detached_html_document();
    assert!(host.append_child(document, target));

    let engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/page.html").unwrap();
    let cssom_base_url = url::Url::parse("https://example.test/cssom/").unwrap();
    engine.set_inline_style_base_url_with_host(&host, target, cssom_base_url);
    set_test_inline_render_text(&mut host, target, "background-image: url(icon.png);");
    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(document),
        1
    );

    assert!(host.append_child(detached_document, target));
    engine.migrate_inline_style_metadata_subtree_with_host(&host, target);

    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(document),
        0,
        "old document world must not retain moved inline style metadata"
    );
    assert_eq!(
        engine.inline_style_base_url_count_for_document_for_test(detached_document),
        1
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "background-image",
            None,
            &FullStyleWorldSnapshot::default(),
            None,
        ),
        Some("url(\"https://example.test/cssom/icon.png\")".into())
    );
}

fn set_test_inline_render_text(host: &mut DomHost, node: DomHandle, text: &str) {
    assert!(host.set_attribute(node, "style", ""));
    host.replace_style_attribute_source(
        node,
        Box::new(crate::css_style::CssomStyleAttribute::with_text(
            Default::default(),
            String::new(),
            Some(text.to_owned()),
            true,
        )),
    );
}

#[test]
fn detached_shadow_adopted_stylesheet_change_uses_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let active = host.create_element("main");
    let detached_document = host.create_detached_html_document();
    let detached_host = host.create_element("section");
    let detached_shadow_root = host
        .attach_shadow_root(detached_host, "open")
        .expect("detached section should host a shadow root");
    let detached = host.create_element("span");
    assert!(host.append_child(document, active));
    assert!(host.append_child(detached_document, detached_host));
    assert!(host.append_child(detached_shadow_root, detached));
    assert!(host.set_attribute(detached, "class", "target"));

    let mut engine = MoliStyleEngine::new();
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let first_source = StyloStylesheetSource::new(
        ".target { color: rgb(1, 2, 3); }".to_owned(),
        document_url.clone(),
    );
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        detached_shadow_root,
        vec![first_source.clone()],
    );
    let detached_shadow_sources =
        engine.shadow_root_adopted_style_sheet_sources_with_host(&host, detached_shadow_root);
    assert_eq!(
        detached_shadow_sources[0].serialized_css_text().as_ref(),
        ".target { color: rgb(1, 2, 3); }"
    );
    let mut detached_inputs = FullStyleWorldSnapshot::default();
    detached_inputs
        .shadow_stylesheet_sources
        .push((detached_shadow_root, detached_shadow_sources));
    let active_inputs = FullStyleWorldSnapshot::default();

    assert!(
        engine
            .computed_style_property_value(
                &host,
                &document_url,
                active,
                "color",
                None,
                &active_inputs,
                None,
            )
            .is_some()
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            detached,
            "color",
            None,
            &detached_inputs,
            None,
        ),
        Some("rgb(1, 2, 3)".into())
    );

    let second_source =
        StyloStylesheetSource::new(".target { color: rgb(4, 5, 6); }".to_owned(), document_url);
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(
        &host,
        detached_shadow_root,
        vec![second_source],
    );

    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(document),
        1
    );
    assert_eq!(
        engine.computed_style_cache_entry_count_for_document_for_test(detached_document),
        1,
        "the detached shadow world invalidates at observation time"
    );
    assert!(engine.computed_style_cache_contains_handle_for_document_for_test(document, active));
    assert!(
        engine.computed_style_cache_contains_handle_for_document_for_test(
            detached_document,
            detached
        )
    );
    assert_only_source_document_is_dirty(&engine, detached_document, document);
}
