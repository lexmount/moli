use super::*;

fn linked_source_text(
    engine: &MoliStyleEngine,
    host: &DomHost,
    owner: DomHandle,
) -> Option<String> {
    engine
        .linked_stylesheet_source_for_owner_with_host(host, owner)
        .map(|source| source.serialized_css_text().to_string())
}

#[test]
fn adopting_inert_template_styles_does_not_prepare_unused_sources() {
    let mut host = test_host();
    let template = host.create_element("template");
    let contents = host.parser_template_contents_handle(template).unwrap();
    let style = host.create_element("style");
    assert!(host.set_text_content(style, "span { color: blue; }"));
    assert!(host.append_child(contents, style));
    let destination = host.create_detached_html_document();
    let mut engine = MoliStyleEngine::new();
    let (_, changes) = host
        .adopt_node_with_stylesheet_owner_changes(destination, template)
        .unwrap();
    engine.apply_stylesheet_owner_changes_with_host(&host, &changes);
    assert!(engine.owner_style_sheet_processing_source(style).is_none());

    // Activating the same element later must still prepare its source.
    let insertion = host.append_child_effects(destination, style);
    engine.apply_stylesheet_owner_changes_with_host(&host, insertion.stylesheet_owners().changes());
    assert_eq!(
        engine
            .owner_style_sheet_processing_source(style)
            .unwrap()
            .css_text(),
        "span { color: blue; }"
    );
}

#[test]
fn style_element_initialization_waits_for_parser_and_preserves_prepared_sources() {
    let mut host = test_host();
    let document = host.document_handle();
    let construction = moli_dom::native::ParserConstruction::default();
    let owner = construction.create_element(
        &mut host,
        document,
        "style".into(),
        "http://www.w3.org/1999/xhtml".into(),
        None,
    );
    assert!(host.set_text_content(owner, ".first { color: red; }"));
    let insertion = host.append_child_effects(document, owner);
    let mut engine = MoliStyleEngine::new();
    engine.apply_stylesheet_owner_changes_with_host(&host, insertion.stylesheet_owners().changes());
    engine.initialize_style_element_sources_with_host(&host, document);
    assert!(engine.owner_style_sheet_processing_source(owner).is_none());

    let completion = construction.finish_children(&mut host, owner);
    construction.finish();
    engine
        .apply_stylesheet_owner_changes_with_host(&host, completion.stylesheet_owners().changes());
    let first = engine.owner_style_sheet_processing_source(owner).unwrap();
    assert_eq!(first.css_text(), ".first { color: red; }");
    engine.initialize_style_element_sources_with_host(&host, document);
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &engine.owner_style_sheet_processing_source(owner).unwrap(),
    ));

    let change = host.set_text_content_effects(owner, ".second { color: blue; }");
    engine.apply_stylesheet_owner_changes_with_host(&host, change.stylesheet_owners().changes());
    let second = engine.owner_style_sheet_processing_source(owner).unwrap();
    assert_eq!(second.css_text(), ".second { color: blue; }");
    assert!(!std::sync::Arc::ptr_eq(&first, &second));
}

#[test]
fn styles_in_documents_without_browsing_context_follow_owner_lifecycle() {
    let mut host = test_host();
    let document = host.create_detached_html_document();
    let style = host.create_element("style");
    assert!(host.set_text_content(style, ".first { color: red; }"));
    let mut engine = MoliStyleEngine::new();
    let insertion = host.append_child_effects(document, style);
    assert!(!host.is_connected(style));
    engine.apply_stylesheet_owner_changes_with_host(&host, insertion.stylesheet_owners().changes());
    assert_eq!(
        engine
            .owner_style_sheet_text_with_host(&host, style)
            .as_deref(),
        Some(".first { color: red; }"),
    );

    let change = host.set_text_content_effects(style, ".second { color: blue; }");
    engine.apply_stylesheet_owner_changes_with_host(&host, change.stylesheet_owners().changes());
    assert_eq!(
        engine
            .owner_style_sheet_text_with_host(&host, style)
            .as_deref(),
        Some(".second { color: blue; }"),
    );
    let removal = host.remove_child_effects(document, style);
    engine.apply_stylesheet_owner_changes_with_host(&host, removal.stylesheet_owners().changes());
    assert!(engine.owner_style_sheet_processing_source(style).is_none());
}

#[test]
fn disabled_author_styles_use_stylo_author_origin_gate() {
    let mut host = test_host();
    host.reset_html_document_shell();
    let body = host.document_body_handle().expect("test body");
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "class", "styled"));
    assert!(host.set_attribute(target, "hidden", ""));
    assert!(host.set_attribute(target, "style", "color: rgb(1, 2, 3); display: none"));
    assert!(host.append_child(body, target));

    let document_url = url::Url::parse("https://example.test/").expect("test URL");
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            ".styled { color: rgb(4, 5, 6); display: none; }".into(),
            document_url.clone(),
        ));
    let engine = MoliStyleEngine::new_with_author_styles_disabled(true);

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            target,
            "display",
            None,
            &inputs,
            None,
        ),
        Some("block".to_owned()),
        "UA styles must remain active while stylesheet and style-attribute declarations are ignored"
    );
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
        Some("rgb(0, 0, 0)".to_owned())
    );

    let svg = host
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg:svg")
        .expect("SVG root");
    let rect = host
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg:rect")
        .expect("SVG rect");
    assert!(host.set_attribute(rect, "display", "none"));
    assert!(host.set_attribute(rect, "fill", "red"));
    assert!(host.append_child(svg, rect));
    assert!(host.append_child(body, svg));

    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            rect,
            "display",
            None,
            &inputs,
            None,
        ),
        Some("inline".to_owned()),
        "SVG presentation attributes must follow the author-style policy"
    );
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            rect,
            "fill",
            None,
            &inputs,
            None,
        ),
        Some("rgb(0, 0, 0)".to_owned())
    );

    let table = host.create_element("table");
    let row = host.create_element("tr");
    let cell = host.create_element("td");
    assert!(host.set_attribute(table, "cellpadding", "17"));
    assert!(host.append_child(row, cell));
    assert!(host.append_child(table, row));
    assert!(host.append_child(body, table));
    assert_eq!(
        engine.computed_style_property_value(
            &host,
            &document_url,
            cell,
            "padding-left",
            None,
            &inputs,
            None,
        ),
        Some("0px".to_owned()),
        "legacy HTML presentation hints must follow the author-style policy"
    );
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
fn linked_stylesheet_source_is_available_in_its_detached_document_until_removed() {
    let mut host = test_host();
    let document = host.create_detached_html_document();
    let link = host.create_element("link");
    assert!(host.set_attribute(link, "rel", "stylesheet"));
    assert!(host.set_attribute(link, "href", "detached.css"));
    assert!(host.append_child(document, link));

    let url = url::Url::parse("https://example.test/detached.css").unwrap();
    let mut engine = MoliStyleEngine::new();
    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &url,
        StyloStylesheetSource::new(".detached-link { color: blue; }".into(), url.clone()),
        &[link],
    );
    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some(".detached-link { color: blue; }"),
    );

    assert!(host.remove_child(document, link));
    assert_eq!(linked_source_text(&engine, &host, link), None);
}

#[test]
fn owner_stylesheet_source_moves_to_current_owner_document_world() {
    let mut host = test_host();
    let document = host.document_handle();
    let detached_document = host.create_detached_html_document();
    let mut engine = MoliStyleEngine::new();

    let owner = host.create_element("style");
    let css = ".moved-owner { color: green; }";
    assert!(host.append_child(document, owner));
    engine.set_owner_style_sheet_text_with_host(&host, owner, css.into());
    let old_world = engine.world_for_document(document);
    assert_eq!(
        old_world.owner_style_sheet_sources.borrow().text(owner),
        Some(css)
    );

    assert!(host.append_child(detached_document, owner));
    engine.set_owner_style_sheet_text_with_host(&host, owner, css.into());
    assert_eq!(
        old_world.owner_style_sheet_sources.borrow().text(owner),
        None
    );
    assert_eq!(
        engine
            .owner_style_sheet_text_with_host(&host, owner)
            .as_deref(),
        Some(css)
    );
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
    assert_ne!(first_source.source_id(), second_source.source_id());
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

    assert_eq!(linked_source_text(&engine, &host, link), None);

    engine.install_linked_stylesheet_source_for_owners_for_test(
        &host,
        &new_url,
        StyloStylesheetSource::new("section { color: green; }".to_owned(), new_url.clone()),
        &[link],
    );

    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some("section { color: green; }")
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

    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some(".old { color: red; }")
    );

    let effects = host.set_attribute_effects(link, "href", "new.css");
    engine.apply_stylesheet_owner_changes_with_host(&host, effects.stylesheet_owners().changes());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);

    assert_eq!(
        linked_source_text(&engine, &host, link),
        None,
        "the old loaded stylesheet must not remain bound after href changes"
    );

    engine.record_stylesheet_source_for_url_for_document_for_test(
        document,
        &new_url,
        StyloStylesheetSource::new(".new { color: blue; }".into(), new_url.clone()),
    );
    assert_eq!(
        linked_source_text(&engine, &host, link),
        None,
        "recording a resource by URL must not infer an owner binding from live href"
    );
    assert!(engine.install_recorded_linked_stylesheet_source_for_test(&host, link, &new_url,));
    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some(".new { color: blue; }")
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
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);

    assert_eq!(
        engine.linked_stylesheet_owner_registry_counts_for_document_for_test(document),
        (1, 1),
        "media changes update the installed sheet instead of detaching its load/source binding"
    );
    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some("main { color: green; }")
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

    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some(".old-base { color: red; }")
    );

    assert!(host.set_attribute(base, "href", "/theme/"));
    let effects = [StyleMutationEffect::Attribute {
        element: base,
        namespace: None,
        name: "href".into(),
        old_value: Some("/assets/".into()),
        new_value: Some("/theme/".into()),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);

    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some(".old-base { color: red; }"),
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

    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some(".old-base { color: red; }")
    );

    let base = host.create_element("base");
    assert!(host.set_attribute(base, "href", "/assets/"));
    assert!(host.insert_before(document, base, Some(link)));
    let effects = [StyleMutationEffect::ChildList {
        parent: document,
        added_nodes: vec![base],
        removed_nodes: vec![],
        previous_sibling: None,
        next_sibling: Some(link),
    }];
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &effects, &media);

    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some(".old-base { color: red; }"),
        "inserting a document base must not re-key an already processed link"
    );
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
fn clearing_adopted_sheets_removes_document_and_shadow_sources() {
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

    engine.set_document_adopted_style_sheet_sources(document, Vec::new());
    engine.set_shadow_root_adopted_style_sheet_sources_with_host(&host, shadow_root, Vec::new());

    assert_eq!(
        engine.adopted_style_sheet_source_owner_counts_for_document_for_test(document),
        (0, 0)
    );
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
}

#[test]
fn disabling_linked_stylesheet_owner_removes_source_binding() {
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

    assert_eq!(
        linked_source_text(&engine, &host, link).as_deref(),
        Some(".linked { color: black; }")
    );

    let effects = host.set_attribute_effects(link, "disabled", "");
    engine.apply_stylesheet_owner_changes_with_host(&host, effects.stylesheet_owners().changes());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);
    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    engine.invalidate_for_mutations(&host, &style_effects, &media);
    assert_eq!(linked_source_text(&engine, &host, link), None);
}

#[test]
fn layout_metric_invalidation_uses_applicable_presentation_hints() {
    const SVG: &str = "http://www.w3.org/2000/svg";
    let mut host = test_host();
    let div = host.create_element("div");
    let table = host.create_element("table");
    let textarea = host.create_element("textarea");
    let link = host.create_element("link");
    let group = host.create_element_ns(Some(SVG), "g").unwrap();
    let gradient = host.create_element_ns(Some(SVG), "linearGradient").unwrap();
    let pattern = host.create_element_ns(Some(SVG), "pattern").unwrap();

    for (element, name, expected) in [
        (div, "style", true),
        (div, "fill", false),
        (div, "cellpadding", false),
        (table, "cellpadding", true),
        (textarea, "cols", true),
        (textarea, "rows", true),
        (link, "href", false),
        (group, "fill", true),
        (group, "r", false),
        (gradient, "gradientTransform", true),
        (gradient, "gradienttransform", false),
        (pattern, "patternTransform", true),
        (pattern, "patterntransform", false),
    ] {
        let native = host.set_attribute_effects(element, name, "1");
        let effects = StyleMutationEffect::from_dom_mutation_effects(&native);
        let [effect] = effects.as_slice() else {
            panic!("expected one style mutation for {name}");
        };
        assert_eq!(effect.affects_layout_metric(&host), expected, "{name}");
    }
}

#[test]
fn inapplicable_presentation_attributes_skip_restyle_without_selector_dependencies() {
    let mut host = test_host();
    let document = host.document_handle();
    let style = host.create_element("style");
    let target = host.create_element("div");
    assert!(host.append_child(document, style));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    engine.set_owner_style_sheet_text_with_host(&host, style, ".headline { color: red; }".into());
    engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
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

    let media = crate::protocol_types::EmulatedMediaOverrides::default();
    let generation = engine.style_invalidation_generation_for_document_for_test(document);
    for name in [
        "data-state",
        "x",
        "r",
        "transform",
        "color",
        "width",
        "cellpadding",
        "face",
        "text",
        "nowrap",
        "cols",
        "rows",
        "size",
    ] {
        let effects = host.set_attribute_effects(target, name, "1");
        let effects = StyleMutationEffect::from_dom_mutation_effects(&effects);
        engine.invalidate_for_mutations(&host, &effects, &media);
        assert_eq!(
            engine.pending_style_invalidation_work_item_count_for_document_for_test(document),
            0,
            "{name} should not queue style invalidation on a div",
        );
        engine.drain_pending_style_invalidations_for_document_for_test(&host, document);
        assert_eq!(
            engine.style_invalidation_generation_for_document_for_test(document),
            generation,
            "{name} should leave cached styles valid",
        );
    }
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
    let document_url = url::Url::parse("https://example.test/").unwrap();
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .push(StyloStylesheetSource::new(
            "script[async] { color: red; }".into(),
            document_url.clone(),
        ));
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
        namespace: None,
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
fn namespace_aware_mutation_preserves_the_dom_attribute_name() {
    let mut host = test_host();
    let element = host.create_element("div");
    for namespace in [Some("urn:test"), None] {
        let native = host.set_attribute_ns_effects(
            element,
            namespace,
            None,
            "DATA-State",
            "DATA-State",
            "marker",
        );
        let effects = StyleMutationEffect::from_dom_mutation_effects(&native);
        assert!(matches!(
            effects.as_slice(),
            [StyleMutationEffect::Attribute {
                element: target,
                namespace: actual_namespace,
                name,
                old_value: None,
                new_value: Some(value),
            }] if *target == element && actual_namespace.as_deref() == namespace
                && name == "DATA-State" && value == "marker"
        ));
    }
}
#[test]
fn typed_attribute_style_effect_captures_exact_new_value_without_observer_records() {
    let mut host = test_host();
    let element = host.create_element("div");
    assert!(host.append_child(host.document_handle(), element));
    let effects = host.set_attribute_effects(element, "class", "marker target");
    assert!(effects.observer_records().records().is_empty());
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);

    assert!(style_effects.iter().any(|effect| matches!(
        effect,
        StyleMutationEffect::Attribute {
            element: target,
            namespace: None,
            name,
            old_value,
            new_value,
        } if *target == element
            && name == "class"
            && old_value.is_none()
            && new_value.as_deref() == Some("marker target")
    )));
}
#[test]
fn typed_attribute_style_effect_normalizes_html_attribute_name() {
    let mut host = test_host();
    let element = host.create_element("div");
    assert!(host.append_child(host.document_handle(), element));
    let effects = host.set_attribute_effects(element, "CLASS", "marker");
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);

    assert!(style_effects.iter().any(|effect| matches!(
        effect,
        StyleMutationEffect::Attribute {
            element: target,
            namespace: None,
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
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(&effects);

    assert!(style_effects.iter().any(|effect| matches!(
        effect,
        StyleMutationEffect::Attribute {
            element: target,
            namespace: Some(namespace),
            name,
            old_value,
            new_value,
        } if *target == element
            && namespace.as_ref() == "urn:test"
            && name == "DATA-State"
            && old_value.is_none()
            && new_value.as_deref() == Some("marker")
    )));
}

#[test]
fn document_stylesheet_dirty_mark_advances_only_the_source_generation() {
    let mut engine = MoliStyleEngine::new();
    let host = test_host();
    let document = host.document_handle();

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        0
    );
    engine.mark_document_stylesheet_set_dirty(document);

    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        0
    );
    assert_eq!(
        engine.source_set_generation_for_document_for_test(document),
        1
    );
}
