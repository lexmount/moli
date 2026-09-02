use super::*;
use style::values::computed::font::FontFamilyNameSyntax;

fn stylesheet_contents(stylesheet: &LiveStylesheet) -> ServoArc<StylesheetContents> {
    let stylo_stylesheet = stylesheet.stylesheet();
    let guard = stylo_stylesheet.shared_lock.read();
    stylo_stylesheet.contents.read_with(&guard).clone()
}

fn native_rule_css_text(stylesheet: &LiveStylesheet, path: &[usize]) -> String {
    stylesheet
        .native_rule_at_path(path)
        .expect("native rule path should resolve")
        .css_text(&stylesheet.stylesheet.shared_lock)
}

fn serialized_stylesheet_text(stylesheet: &LiveStylesheetRef) -> StdArc<str> {
    crate::style_engine::StyloStylesheetSource::from_live_stylesheet(stylesheet)
        .serialized_css_text()
}

fn style_rule_seed_declaration_text(stylesheet: &LiveStylesheet, path: &[usize]) -> String {
    let Some(LiveStylesheetRuleWrapperSeed::Style {
        declaration_text, ..
    }) = stylesheet.rule_wrapper_seed_at_path(path)
    else {
        panic!("style rule seed should resolve at {path:?}");
    };
    declaration_text
}

#[test]
fn eof_open_var_cssom_override_is_shared_and_cleared_by_native_mutation() {
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let base_url = url::Url::parse("https://example.test/styles/open-var.css").unwrap();
    let source = ".target { color: red; width: var(--prop";
    let first = registry.create_inline_with_shared_initial_contents(
        source,
        base_url.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let second = registry.create_inline_with_shared_initial_contents(
        source,
        base_url,
        QuirksMode::NoQuirks,
        shared_lock,
    );

    assert_eq!(
        style_rule_seed_declaration_text(&first, &[0]),
        "color: red; width: var(--prop;"
    );
    assert_eq!(
        style_rule_seed_declaration_text(&second, &[0]),
        "color: red; width: var(--prop;"
    );
    assert_eq!(
        native_rule_css_text(&first, &[0]),
        ".target { color: red; width: var(--prop); }"
    );

    first
        .set_style_rule_declarations(&[0], "width: var(--updated);")
        .unwrap();
    assert_eq!(
        style_rule_seed_declaration_text(&first, &[0]),
        "width: var(--updated);"
    );
    assert_eq!(
        style_rule_seed_declaration_text(&second, &[0]),
        "color: red; width: var(--prop;"
    );

    first.replace_from_text(".replacement { height: var(--replacement");
    assert_eq!(
        style_rule_seed_declaration_text(&first, &[0]),
        "height: var(--replacement;"
    );
}

#[test]
fn native_import_edges_own_distinct_children_and_propagate_cascade_changes() {
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let base_url = url::Url::parse("https://example.test/styles/root.css").unwrap();
    let root = registry.create(
        "@import './shared.css'; @import './shared.css'; .root { color: red; }",
        base_url,
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        shared_lock,
    );
    let requests = root.pending_import_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url, requests[1].url);
    assert_ne!(requests[0].edge_id, requests[1].edge_id);

    let revision = root.contents_revision();
    let first = registry
        .install_import_response(
            root.id(),
            revision,
            requests[0].edge_id,
            ".first { color: blue; }",
            requests[0].url.clone(),
            true,
            true,
        )
        .unwrap();
    let second = registry
        .install_import_response(
            root.id(),
            revision,
            requests[1].edge_id,
            ".second { color: green; }",
            requests[1].url.clone(),
            true,
            true,
        )
        .unwrap();

    assert_ne!(first.id(), second.id());
    assert!(!ServoArc::ptr_eq(&first.stylesheet(), &second.stylesheet()));
    assert_eq!(
        root.import_edge_state(requests[0].edge_id),
        Some(LiveStylesheetImportState::Loaded { successful: true })
    );
    assert_eq!(
        root.import_edge_state(requests[1].edge_id),
        Some(LiveStylesheetImportState::Loaded { successful: true })
    );

    let root_generation = root.cascade_generation();
    first
        .insert_rule(".first-only { display: block; }", 1)
        .unwrap();
    assert!(root.cascade_generation() > root_generation);
    assert!(serialized_stylesheet_text(&first).contains("first-only"));
    assert!(!serialized_stylesheet_text(&second).contains("first-only"));
}

#[test]
fn parsed_import_template_outlives_its_registry_wrapper_for_owner_cloning() {
    let registry = LiveStylesheetRegistry::default();
    let base_url = url::Url::parse("https://example.test/styles/root.css").unwrap();
    let template = registry.create(
        "@import './child.css'; .root { color: red; }",
        base_url.clone(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    let parsed_stylesheet = template.stylesheet();
    drop(template);
    assert_eq!(registry.live_entry_count_for_test(), 0);

    let owner = registry.create_from_parsed_stylesheet(
        &parsed_stylesheet,
        base_url,
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
    );

    assert_eq!(owner.pending_import_requests().len(), 1);
    assert!(serialized_stylesheet_text(&owner).contains("@import"));
}

#[test]
fn removed_import_child_cannot_invalidate_parent_and_stale_completion_is_rejected() {
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let base_url = url::Url::parse("https://example.test/styles/root.css").unwrap();
    let root = registry.create(
        "@import './child.css'; .root { color: red; }",
        base_url,
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        shared_lock,
    );
    let request = root.pending_import_requests().pop().unwrap();
    let original_revision = root.contents_revision();
    let child = registry
        .install_import_response(
            root.id(),
            original_revision,
            request.edge_id,
            ".child { color: blue; }",
            request.url.clone(),
            true,
            true,
        )
        .unwrap();

    root.delete_rule(0).unwrap();
    let generation_after_delete = root.cascade_generation();
    child
        .insert_rule(".detached { display: block; }", 1)
        .unwrap();
    assert_eq!(root.cascade_generation(), generation_after_delete);

    root.insert_rule("@import './child.css';", 0).unwrap();
    assert!(
        registry
            .install_import_response(
                root.id(),
                original_revision,
                request.edge_id,
                ".stale { color: black; }",
                request.url,
                true,
                true,
            )
            .is_none()
    );
}

#[test]
fn descendant_import_replacement_invalidates_a_root_graph_snapshot() {
    let registry = LiveStylesheetRegistry::default();
    let root = registry.create(
        "@import './child.css';",
        url::Url::parse("https://example.test/styles/root.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    let root_request = root.pending_import_requests().pop().unwrap();
    let child = registry
        .install_import_response(
            root.id(),
            root.contents_revision(),
            root_request.edge_id,
            "@import './leaf.css';",
            root_request.url,
            true,
            true,
        )
        .unwrap();
    let root_revision = root.contents_revision();
    let stale_import_generation = root.import_generation();
    let stale_leaf_request = child.pending_import_requests().pop().unwrap();

    child.delete_rule(0).unwrap();
    child.insert_rule("@import './leaf.css';", 0).unwrap();

    assert_eq!(root.contents_revision(), root_revision);
    assert!(root.import_generation() > stale_import_generation);
    assert_eq!(child.pending_import_requests().len(), 1);
    assert_eq!(
        registry.install_import_graph(
            root.id(),
            root_revision,
            stale_import_generation,
            &[LiveStylesheetImportResponse {
                request_url: stale_leaf_request.url.clone(),
                response_url: stale_leaf_request.url,
                css_text: ".stale { color: red; }".to_owned(),
                successful: true,
                origin_clean: true,
            }],
            Some(root.base_url()),
        ),
        None
    );
    assert!(child.pending_import_requests().iter().all(|request| {
        child.import_edge_state(request.edge_id) == Some(LiveStylesheetImportState::Pending)
    }));
}

#[test]
fn import_graph_uses_each_response_url_as_the_nested_parser_base() {
    let registry = LiveStylesheetRegistry::default();
    let root = registry.create(
        "@import './redirected.css';",
        url::Url::parse("https://example.test/assets/root.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    let responses = vec![
        LiveStylesheetImportResponse {
            request_url: url::Url::parse("https://example.test/assets/redirected.css").unwrap(),
            response_url: url::Url::parse("https://cdn.example.test/final/child.css").unwrap(),
            css_text: "@import './leaf.css'; .child { color: blue; }".to_owned(),
            successful: true,
            origin_clean: true,
        },
        LiveStylesheetImportResponse {
            request_url: url::Url::parse("https://cdn.example.test/final/leaf.css").unwrap(),
            response_url: url::Url::parse("https://cdn.example.test/final/leaf.css").unwrap(),
            css_text: ".leaf { color: green; }".to_owned(),
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
        Some(true)
    );
    let NativeStylesheetRule::Css(CssRule::Import(child_rule)) =
        root.native_rule_at_path(&[0]).unwrap()
    else {
        panic!("root rule must remain a native import");
    };
    let child = root.imported_child_for_rule(&child_rule).unwrap();
    assert!(child.pending_import_requests().is_empty());
    let NativeStylesheetRule::Css(CssRule::Import(leaf_rule)) =
        child.native_rule_at_path(&[0]).unwrap()
    else {
        panic!("child rule must remain a native import");
    };
    let leaf = child.imported_child_for_rule(&leaf_rule).unwrap();
    assert_eq!(
        leaf.base_url().as_str(),
        "https://cdn.example.test/final/leaf.css"
    );
    assert!(serialized_stylesheet_text(&leaf).contains(".leaf"));
}

#[test]
fn deep_import_graph_walks_and_generation_propagation_are_iterative() {
    const DEPTH: usize = 512;

    let registry = LiveStylesheetRegistry::default();
    let root = registry.create(
        "@import './level-0.css';",
        url::Url::parse("https://example.test/styles/root.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    let mut parent = root.clone();

    for level in 0..DEPTH {
        let request = parent
            .pending_import_requests()
            .pop()
            .expect("each intermediate sheet must have one pending import");
        let css_text = if level + 1 == DEPTH {
            "@import './terminal.css';".to_owned()
        } else {
            format!("@import './level-{}.css';", level + 1)
        };
        parent = registry
            .install_import_response(
                parent.id(),
                parent.contents_revision(),
                request.edge_id,
                &css_text,
                request.url,
                true,
                true,
            )
            .expect("intermediate import response must install");
    }

    let pending = root.pending_import_requests_in_graph();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].url.path(), "/styles/terminal.css");
    let generation_before_terminal = root.import_generation();
    assert_eq!(
        registry.install_import_graph(
            root.id(),
            root.contents_revision(),
            generation_before_terminal,
            &[LiveStylesheetImportResponse {
                request_url: pending[0].url.clone(),
                response_url: pending[0].url.clone(),
                css_text: ".terminal { color: green; }".to_owned(),
                successful: true,
                origin_clean: true,
            }],
            Some(root.base_url()),
        ),
        Some(true)
    );
    assert!(root.pending_import_requests_in_graph().is_empty());
    assert!(root.import_generation() > generation_before_terminal);
}

#[test]
fn data_imports_are_native_children_and_cycles_finish_as_empty_sheets() {
    let registry = LiveStylesheetRegistry::default();
    let data_url = url::Url::parse(
        "data:text/css,@import%20url(%22data:text/css,.leaf%257Bcolor%253Agreen%257D%22)%3B",
    )
    .unwrap();
    let root = registry.create(
        &format!("@import url(\"{data_url}\");"),
        url::Url::parse("https://example.test/root.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    assert_eq!(
        registry.install_import_graph(
            root.id(),
            root.contents_revision(),
            root.import_generation(),
            &[],
            None,
        ),
        Some(true)
    );
    let NativeStylesheetRule::Css(CssRule::Import(child_rule)) =
        root.native_rule_at_path(&[0]).unwrap()
    else {
        panic!("root rule must remain a native import");
    };
    let child = root.imported_child_for_rule(&child_rule).unwrap();
    let NativeStylesheetRule::Css(CssRule::Import(leaf_rule)) =
        child.native_rule_at_path(&[0]).unwrap()
    else {
        panic!("data child rule must remain a native import");
    };
    let leaf = child.imported_child_for_rule(&leaf_rule).unwrap();
    assert!(serialized_stylesheet_text(&leaf).contains(".leaf"));

    let cycle_url = url::Url::parse("https://example.test/cycle.css").unwrap();
    let cycle_root = registry.create(
        "@import './cycle.css';",
        cycle_url.clone(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    assert_eq!(
        registry.install_import_graph(
            cycle_root.id(),
            cycle_root.contents_revision(),
            cycle_root.import_generation(),
            &[],
            Some(&cycle_url),
        ),
        Some(false)
    );
    let NativeStylesheetRule::Css(CssRule::Import(cycle_rule)) =
        cycle_root.native_rule_at_path(&[0]).unwrap()
    else {
        panic!("cycle rule must remain a native import");
    };
    let empty = cycle_root.imported_child_for_rule(&cycle_rule).unwrap();
    assert!(serialized_stylesheet_text(&empty).is_empty());
}

#[test]
fn data_import_ignores_the_declared_mime_type_like_chromium() {
    let registry = LiveStylesheetRegistry::default();
    let root = registry.create(
        "@import url('data:image/png,not-css');",
        url::Url::parse("https://example.test/root.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    let request = root.pending_import_requests().pop().unwrap();

    assert_eq!(
        registry.install_import_graph(
            root.id(),
            root.contents_revision(),
            root.import_generation(),
            &[],
            None,
        ),
        Some(true)
    );
    let NativeStylesheetRule::Css(CssRule::Import(import_rule)) =
        root.native_rule_at_path(&[0]).unwrap()
    else {
        panic!("root rule must remain a native import");
    };
    let child = root.imported_child_for_rule(&import_rule).unwrap();
    assert!(
        serialized_stylesheet_text(&child).is_empty(),
        "the selected data resource loads as CSS, while its invalid rule text parses to an empty sheet"
    );
    assert_eq!(
        root.import_edge_state(request.edge_id),
        Some(LiveStylesheetImportState::Loaded { successful: true })
    );
}

#[test]
fn identical_inline_stylesheets_share_initial_contents_and_copy_on_first_mutation() {
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let base_url = url::Url::parse("https://example.test/styles/inline.css").unwrap();
    let css_text = concat!(
        "@font-face { font-family: SharedFace; src: local(Arial); } ",
        ".subject { color: red; }"
    );

    reset_live_stylesheet_parse_count_for_test();
    let first = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let second = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url,
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );

    assert_ne!(first.id(), second.id());
    assert!(!ServoArc::ptr_eq(&first.stylesheet(), &second.stylesheet()));
    assert!(ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&second)
    ));
    assert_eq!(live_stylesheet_parse_count_for_test(), 1);

    let first_font_identity = first
        .font_faces(
            crate::style_engine::StyloStyleEnvironment::default(),
            crate::style_engine::StyleViewport::default(),
        )
        .all_rules[0]
        .rule_identity;
    first
        .insert_rule(".first-only { color: blue; }", 2)
        .unwrap();

    assert!(!ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&second)
    ));
    assert!(serialized_stylesheet_text(&first).contains("first-only"));
    assert!(!serialized_stylesheet_text(&second).contains("first-only"));
    assert_eq!(live_stylesheet_parse_count_for_test(), 1);
    assert_eq!(
        first
            .font_faces(
                crate::style_engine::StyloStyleEnvironment::default(),
                crate::style_engine::StyleViewport::default(),
            )
            .all_rules[0]
            .rule_identity,
        first_font_identity,
        "copy-on-write must not change an untouched native font-face identity"
    );

    drop(second);
    let reparsed = registry.create_inline_with_shared_initial_contents(
        css_text,
        url::Url::parse("https://example.test/styles/inline.css").unwrap(),
        QuirksMode::NoQuirks,
        shared_lock,
    );
    assert_eq!(live_stylesheet_parse_count_for_test(), 2);
    assert!(!ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&reparsed)
    ));
}

#[test]
fn inline_contents_cache_excludes_empty_imported_and_different_context_sources() {
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let first_base = url::Url::parse("https://example.test/first/").unwrap();
    let second_base = url::Url::parse("https://example.test/second/").unwrap();

    reset_live_stylesheet_parse_count_for_test();
    let empty_first = registry.create_inline_with_shared_initial_contents(
        "",
        first_base.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let empty_second = registry.create_inline_with_shared_initial_contents(
        "",
        first_base.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    assert!(!ServoArc::ptr_eq(
        &stylesheet_contents(&empty_first),
        &stylesheet_contents(&empty_second)
    ));

    let imported_first = registry.create_inline_with_shared_initial_contents(
        "@import url(shared.css);",
        first_base.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let imported_second = registry.create_inline_with_shared_initial_contents(
        "@import url(shared.css);",
        first_base.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    assert!(!ServoArc::ptr_eq(
        &stylesheet_contents(&imported_first),
        &stylesheet_contents(&imported_second)
    ));

    let contextual_first = registry.create_inline_with_shared_initial_contents(
        ".subject { background: url(image.png); }",
        first_base,
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let contextual_second = registry.create_inline_with_shared_initial_contents(
        ".subject { background: url(image.png); }",
        second_base,
        QuirksMode::NoQuirks,
        shared_lock,
    );
    assert!(!ServoArc::ptr_eq(
        &stylesheet_contents(&contextual_first),
        &stylesheet_contents(&contextual_second)
    ));
    assert_eq!(live_stylesheet_parse_count_for_test(), 6);
}

#[test]
fn registry_is_a_weak_index_and_serialization_is_revision_scoped() {
    reset_live_stylesheet_css_text_projection_count_for_test();
    let registry = LiveStylesheetRegistry::default();
    let stylesheet = registry.create(
        ".subject { color: red; }",
        url::Url::parse("https://example.test/styles/main.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::No,
        SharedRwLock::new(),
    );
    let id = stylesheet.id();

    let source = crate::style_engine::StyloStylesheetSource::from_live_stylesheet(&stylesheet);
    let first = serialized_stylesheet_text(&stylesheet);
    let second = source.serialized_css_text();
    assert!(StdArc::ptr_eq(&first, &second));
    assert_eq!(live_stylesheet_css_text_projection_count_for_test(), 1);
    assert_eq!(registry.get(id).unwrap().id(), id);
    assert_eq!(stylesheet.cascade_generation(), 1);

    stylesheet.replace_from_text(".subject { color: blue; }");
    let replacement_source =
        crate::style_engine::StyloStylesheetSource::from_live_stylesheet(&stylesheet);
    let replacement = serialized_stylesheet_text(&stylesheet);
    let replacement_from_source = replacement_source.serialized_css_text();
    assert!(!StdArc::ptr_eq(&first, &replacement));
    assert!(StdArc::ptr_eq(&replacement, &replacement_from_source));
    assert!(replacement.contains("blue"));
    assert_eq!(live_stylesheet_css_text_projection_count_for_test(), 2);
    assert_eq!(stylesheet.cascade_generation(), 2);

    drop(stylesheet);
    assert!(registry.get(id).is_none());
    assert_eq!(registry.live_entry_count_for_test(), 0);
}

#[test]
fn wrapper_lease_can_release_and_replace_the_live_stylesheet_before_finalization() {
    let registry = LiveStylesheetRegistry::default();
    let first = registry.create(
        ".first { color: red; }",
        url::Url::parse("https://example.test/first.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::No,
        SharedRwLock::new(),
    );
    let first_id = first.id();
    let (lease_id, lease) = registry.create_wrapper_lease(Rc::clone(&first));
    drop(first);
    assert!(registry.get(first_id).is_some());

    assert!(registry.replace_wrapper_lease(lease_id, None));
    assert!(registry.get(first_id).is_none());

    let second = registry.create(
        ".second { color: blue; }",
        url::Url::parse("https://example.test/second.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::No,
        SharedRwLock::new(),
    );
    let second_id = second.id();
    assert!(registry.replace_wrapper_lease(lease_id, Some(second)));
    assert!(registry.get(second_id).is_some());

    lease.borrow_mut().take();
    drop(lease);
    assert!(registry.get(second_id).is_none());
}

#[test]
fn rule_wrapper_lease_tracks_native_identity_across_copy_on_write_and_detach() {
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let base_url = url::Url::parse("https://example.test/styles/inline.css").unwrap();
    let css_text = ".shared { color: red; } @media screen { .nested { color: blue; } }";
    let first = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let second = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url,
        QuirksMode::NoQuirks,
        shared_lock,
    );
    let first_id = first.id();

    let original_rule = first.native_rule_at_path(&[1, 0]).unwrap();
    assert!(original_rule.ptr_eq(&second.native_rule_at_path(&[1, 0]).unwrap()));
    let (lease_id, lease) = registry
        .bind_rule_wrapper(None, &first, vec![1, 0])
        .unwrap();
    let lease = lease.unwrap();
    assert_eq!(first.rule_wrapper_leases.borrow().len(), 1);

    let (reused_id, replacement_lease) = registry
        .bind_rule_wrapper(Some(lease_id), &first, vec![1, 0])
        .unwrap();
    assert_eq!(reused_id, lease_id);
    assert!(replacement_lease.is_none());
    assert_eq!(first.rule_wrapper_leases.borrow().len(), 1);

    first
        .insert_rule(".first-only { color: green; }", 0)
        .unwrap();
    let rebound = registry.rule_wrapper_binding(lease_id).unwrap();
    assert_eq!(rebound.stylesheet_id(), Some(first_id));
    assert_eq!(rebound.path, vec![2, 0]);
    assert!(
        rebound
            .rule()
            .ptr_eq(&first.native_rule_at_path(&[2, 0]).unwrap())
    );
    assert!(!rebound.rule().ptr_eq(&original_rule));
    assert!(original_rule.ptr_eq(&second.native_rule_at_path(&[1, 0]).unwrap()));

    assert!(registry.release_rule_wrapper(lease_id));
    assert!(registry.rule_wrapper_binding(lease_id).is_none());

    drop(first);
    assert!(registry.get(first_id).is_none());
    drop(lease);
}

#[test]
fn parent_rule_replacement_rebinds_root_and_detaches_old_descendants() {
    let registry = LiveStylesheetRegistry::default();
    let stylesheet = registry.create(
        "@media screen { .old { color: red; } }",
        url::Url::parse("https://example.test/styles/replacement.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    let stylesheet_id = stylesheet.id();
    let old_parent = stylesheet.native_rule_at_path(&[0]).unwrap();
    let old_child = stylesheet.native_rule_at_path(&[0, 0]).unwrap();
    let (parent_id, parent_lease) = registry
        .bind_rule_wrapper(None, &stylesheet, vec![0])
        .unwrap();
    let (child_id, child_lease) = registry
        .bind_rule_wrapper(None, &stylesheet, vec![0, 0])
        .unwrap();
    let _parent_lease = parent_lease.unwrap();
    let _child_lease = child_lease.unwrap();

    stylesheet
        .replace_rule("@media print { .new { color: blue; } }", 0)
        .unwrap();

    let parent = registry.rule_wrapper_binding(parent_id).unwrap();
    assert_eq!(parent.stylesheet_id(), Some(stylesheet_id));
    assert_eq!(parent.path, vec![0]);
    assert!(
        parent
            .rule()
            .ptr_eq(&stylesheet.native_rule_at_path(&[0]).unwrap())
    );
    assert!(!parent.rule().ptr_eq(&old_parent));

    let child = registry.rule_wrapper_binding(child_id).unwrap();
    assert_eq!(child.stylesheet_id(), None);
    assert!(child.rule().ptr_eq(&old_child));
    assert_eq!(
        registry
            .retained_rule_wrapper_snapshot_for_detach(child_id)
            .unwrap()
            .css_text,
        ".old { color: red; }"
    );
    assert!(
        registry
            .with_attached_rule_wrapper(child_id, stylesheet_id, |_| ())
            .is_none()
    );
}

#[test]
fn top_level_native_mutations_include_namespace_parser_state() {
    let registry = LiveStylesheetRegistry::default();
    let stylesheet = registry.create(
        ".first { color: red; } .second { color: blue; }",
        url::Url::parse("https://example.test/styles/main.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );

    reset_live_stylesheet_mutation_metrics_for_test();
    let revision = stylesheet.contents_revision();
    assert!(matches!(
        stylesheet.insert_rule(".broken { color: red; } trailing", 1),
        Err(CssRuleInsertError::Syntax)
    ));
    assert_eq!(stylesheet.contents_revision(), revision);
    assert_eq!(
        live_stylesheet_mutation_metrics_for_test(),
        LiveStylesheetMutationMetrics::new()
    );

    stylesheet
        .insert_rule(".inserted { color: green; }", 1)
        .unwrap();
    assert_eq!(
        live_stylesheet_mutation_metrics_for_test().recursive_rule_snapshots,
        0,
        "native insert must not project the inserted rule"
    );
    assert_eq!(
        native_rule_css_text(&stylesheet, &[1]),
        ".inserted { color: green; }"
    );
    stylesheet
        .replace_rule(".replacement { color: black; }", 0)
        .unwrap();
    stylesheet.delete_rule(2).unwrap();
    let metrics = live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 3);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "native mutation reads must not build detached snapshots"
    );

    let namespace_stylesheet = registry.create(
        "",
        url::Url::parse("https://example.test/styles/namespaced.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    namespace_stylesheet
        .insert_rule("@namespace svg url(\"http://www.w3.org/2000/svg\");", 0)
        .unwrap();
    namespace_stylesheet
        .insert_rule("svg|rect { color: green; }", 1)
        .unwrap();
    assert!(matches!(
        namespace_stylesheet
            .insert_rule("@namespace html url(\"http://www.w3.org/1999/xhtml\");", 2,),
        Err(CssRuleInsertError::InvalidState)
    ));
    assert_eq!(
        namespace_stylesheet.delete_rule(0),
        Err(CssRuleInsertError::InvalidState)
    );
    namespace_stylesheet.delete_rule(1).unwrap();
    namespace_stylesheet.delete_rule(0).unwrap();
    assert!(matches!(
        namespace_stylesheet.insert_rule("svg|rect { color: blue; }", 0),
        Err(CssRuleInsertError::Syntax)
    ));
    let metrics = live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_top_level_mutations, 7);
    assert_eq!(
        metrics.recursive_rule_snapshots, 0,
        "namespace-aware mutations must not build detached snapshots"
    );
}

#[test]
fn top_level_native_mutations_preserve_nested_rule_tree() {
    let registry = LiveStylesheetRegistry::default();
    let stylesheet = registry.create(
        "",
        url::Url::parse("https://example.test/styles/nested-insert.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );

    stylesheet
        .insert_rule(
            "div { @media screen { color: red; background-color: green; } }",
            0,
        )
        .unwrap();

    assert_eq!(stylesheet.child_rule_count_at_path(&[0]), Some(1));
    assert_eq!(
        native_rule_css_text(&stylesheet, &[0]),
        "div {\n  @media screen {\n  color: red; background-color: green;\n}\n}"
    );
}

#[test]
fn nested_native_mutations_are_transactional_and_preserve_wrapper_paths() {
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let base_url = url::Url::parse("https://example.test/styles/nested.css").unwrap();
    let css_text = "@media screen { .first { color: red; } .second { color: blue; } }";
    let first = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let second = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url,
        QuirksMode::NoQuirks,
        shared_lock,
    );
    let first_id = first.id();
    let first_rule = first.native_rule_at_path(&[0, 0]).unwrap();
    let second_rule = first.native_rule_at_path(&[0, 1]).unwrap();
    assert!(first_rule.ptr_eq(&second.native_rule_at_path(&[0, 0]).unwrap()));

    let (first_lease_id, first_lease) = registry
        .bind_rule_wrapper(None, &first, vec![0, 0])
        .unwrap();
    let (second_lease_id, second_lease) = registry
        .bind_rule_wrapper(None, &first, vec![0, 1])
        .unwrap();
    let _first_lease = first_lease.unwrap();
    let _second_lease = second_lease.unwrap();

    reset_live_stylesheet_mutation_metrics_for_test();
    let revision = first.contents_revision();
    assert!(matches!(
        first.insert_nested_rule(
            &[0],
            ".broken { color: red; } trailing",
            1,
            CssRuleType::Media.bit(),
            None,
        ),
        Err(CssRuleInsertError::Syntax)
    ));
    assert_eq!(first.contents_revision(), revision);
    assert!(ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&second)
    ));
    assert_eq!(
        live_stylesheet_mutation_metrics_for_test(),
        LiveStylesheetMutationMetrics::new()
    );

    first
        .insert_nested_rule(
            &[0],
            ".inserted { color: green; }",
            1,
            CssRuleType::Media.bit(),
            None,
        )
        .unwrap();
    assert_eq!(
        native_rule_css_text(&first, &[0, 1]),
        ".inserted { color: green; }"
    );
    assert!(!ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&second)
    ));

    let rebound_first = registry.rule_wrapper_binding(first_lease_id).unwrap();
    assert_eq!(rebound_first.path, vec![0, 0]);
    assert_eq!(rebound_first.stylesheet_id(), Some(first_id));
    assert!(
        rebound_first
            .rule()
            .ptr_eq(&first.native_rule_at_path(&[0, 0]).unwrap())
    );
    assert!(!rebound_first.rule().ptr_eq(&first_rule));
    let rebound_second = registry.rule_wrapper_binding(second_lease_id).unwrap();
    assert_eq!(rebound_second.path, vec![0, 2]);
    assert!(
        rebound_second
            .rule()
            .ptr_eq(&first.native_rule_at_path(&[0, 2]).unwrap())
    );
    assert!(!rebound_second.rule().ptr_eq(&second_rule));
    assert_eq!(
        registry
            .with_attached_rule_wrapper(second_lease_id, first_id, |binding| binding.css_text())
            .unwrap(),
        ".second { color: blue; }"
    );
    assert!(
        registry
            .with_attached_rule_wrapper(second_lease_id, second.id(), |_| ())
            .is_none()
    );

    first.delete_nested_rule(&[0], 0).unwrap();
    let detached_first = registry.rule_wrapper_binding(first_lease_id).unwrap();
    assert_eq!(detached_first.stylesheet_id(), None);
    assert!(detached_first.rule().ptr_eq(rebound_first.rule()));
    assert!(
        registry
            .with_attached_rule_wrapper(first_lease_id, first_id, |_| ())
            .is_none()
    );
    let shifted_second = registry.rule_wrapper_binding(second_lease_id).unwrap();
    assert_eq!(shifted_second.path, vec![0, 1]);
    assert!(
        shifted_second
            .rule()
            .ptr_eq(&first.native_rule_at_path(&[0, 1]).unwrap())
    );

    let before_replacement = shifted_second.rule().clone();
    let projections_before_replacement =
        live_stylesheet_mutation_metrics_for_test().recursive_rule_snapshots;
    first
        .replace_nested_rule(
            &[0],
            ".replacement { color: black; }",
            1,
            CssRuleType::Media.bit(),
            None,
        )
        .unwrap();
    assert_eq!(
        live_stylesheet_mutation_metrics_for_test().recursive_rule_snapshots,
        projections_before_replacement,
        "native replacement must not project the replacement subtree"
    );
    assert_eq!(
        native_rule_css_text(&first, &[0, 1]),
        ".replacement { color: black; }"
    );
    let replaced_second = registry.rule_wrapper_binding(second_lease_id).unwrap();
    assert_eq!(replaced_second.path, vec![0, 1]);
    assert!(
        replaced_second
            .rule()
            .ptr_eq(&first.native_rule_at_path(&[0, 1]).unwrap())
    );
    assert!(!replaced_second.rule().ptr_eq(&before_replacement));
    assert_eq!(
        registry
            .with_attached_rule_wrapper(second_lease_id, first_id, |binding| binding.css_text())
            .unwrap(),
        ".replacement { color: black; }"
    );
    assert!(serialized_stylesheet_text(&first).contains("replacement"));
    assert!(serialized_stylesheet_text(&second).contains(".second"));
    assert!(!serialized_stylesheet_text(&second).contains("inserted"));

    let metrics = live_stylesheet_mutation_metrics_for_test();
    assert_eq!(first.contents_revision(), revision + 3);
    assert_eq!(metrics.native_nested_mutations, 3);
    assert_eq!(metrics.native_keyframe_mutations, 0);
    assert_eq!(metrics.recursive_rule_snapshots, 0);
}

#[test]
fn keyframe_native_mutations_are_transactional_and_preserve_wrapper_paths() {
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let base_url = url::Url::parse("https://example.test/styles/keyframes.css").unwrap();
    let css_text = "@keyframes pulse { from { opacity: 0; } to { opacity: 1; } }";
    let first = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let second = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url,
        QuirksMode::NoQuirks,
        shared_lock,
    );
    let first_id = first.id();
    let first_rule = first.native_rule_at_path(&[0, 0]).unwrap();
    let second_rule = first.native_rule_at_path(&[0, 1]).unwrap();
    assert!(first_rule.ptr_eq(&second.native_rule_at_path(&[0, 0]).unwrap()));

    let (first_lease_id, first_lease) = registry
        .bind_rule_wrapper(None, &first, vec![0, 0])
        .unwrap();
    let (second_lease_id, second_lease) = registry
        .bind_rule_wrapper(None, &first, vec![0, 1])
        .unwrap();
    let _first_lease = first_lease.unwrap();
    let _second_lease = second_lease.unwrap();

    reset_live_stylesheet_mutation_metrics_for_test();
    let revision = first.contents_revision();
    assert!(matches!(
        first.insert_keyframe_rule(&[0], "middle { opacity: 0.5; }", 1),
        Err(CssRuleInsertError::Syntax)
    ));
    assert_eq!(first.contents_revision(), revision);
    assert!(ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&second)
    ));
    assert_eq!(
        live_stylesheet_mutation_metrics_for_test(),
        LiveStylesheetMutationMetrics::new()
    );

    first
        .insert_keyframe_rule(&[0], "50% { opacity: 0.5; }", 1)
        .unwrap();
    assert_eq!(
        native_rule_css_text(&first, &[0, 1]),
        "50% { opacity: 0.5; }"
    );
    assert!(!ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&second)
    ));

    let rebound_first = registry.rule_wrapper_binding(first_lease_id).unwrap();
    assert_eq!(rebound_first.path, vec![0, 0]);
    assert_eq!(rebound_first.stylesheet_id(), Some(first_id));
    assert!(
        rebound_first
            .rule()
            .ptr_eq(&first.native_rule_at_path(&[0, 0]).unwrap())
    );
    assert!(!rebound_first.rule().ptr_eq(&first_rule));
    let rebound_second = registry.rule_wrapper_binding(second_lease_id).unwrap();
    assert_eq!(rebound_second.path, vec![0, 2]);
    assert!(
        rebound_second
            .rule()
            .ptr_eq(&first.native_rule_at_path(&[0, 2]).unwrap())
    );
    assert!(!rebound_second.rule().ptr_eq(&second_rule));
    assert_eq!(
        registry
            .with_attached_rule_wrapper(second_lease_id, first_id, |binding| binding.css_text())
            .unwrap(),
        "100% { opacity: 1; }"
    );

    first.delete_keyframe_rule(&[0], 0).unwrap();
    let detached_first = registry.rule_wrapper_binding(first_lease_id).unwrap();
    assert_eq!(detached_first.stylesheet_id(), None);
    assert!(detached_first.rule().ptr_eq(rebound_first.rule()));
    assert!(
        registry
            .with_attached_rule_wrapper(first_lease_id, first_id, |_| ())
            .is_none()
    );
    let shifted_second = registry.rule_wrapper_binding(second_lease_id).unwrap();
    assert_eq!(shifted_second.path, vec![0, 1]);

    let before_replacement = shifted_second.rule().clone();
    let projections_before_replacement =
        live_stylesheet_mutation_metrics_for_test().recursive_rule_snapshots;
    first
        .replace_keyframe_rule(&[0], "100% { opacity: 0.75; }", 1)
        .unwrap();
    assert_eq!(
        live_stylesheet_mutation_metrics_for_test().recursive_rule_snapshots,
        projections_before_replacement,
        "native keyframe replacement must not project the replacement rule"
    );
    assert_eq!(
        native_rule_css_text(&first, &[0, 1]),
        "100% { opacity: 0.75; }"
    );
    let replaced_second = registry.rule_wrapper_binding(second_lease_id).unwrap();
    assert_eq!(replaced_second.path, vec![0, 1]);
    assert!(
        replaced_second
            .rule()
            .ptr_eq(&first.native_rule_at_path(&[0, 1]).unwrap())
    );
    assert!(!replaced_second.rule().ptr_eq(&before_replacement));
    assert_eq!(
        registry
            .with_attached_rule_wrapper(second_lease_id, first_id, |binding| binding.css_text())
            .unwrap(),
        "100% { opacity: 0.75; }"
    );
    assert!(serialized_stylesheet_text(&first).contains("0.75"));
    assert!(serialized_stylesheet_text(&second).contains("opacity: 1"));
    assert!(!serialized_stylesheet_text(&second).contains("50%"));

    let metrics = live_stylesheet_mutation_metrics_for_test();
    assert_eq!(first.contents_revision(), revision + 3);
    assert_eq!(metrics.native_nested_mutations, 0);
    assert_eq!(metrics.native_keyframe_mutations, 3);
    assert_eq!(metrics.recursive_rule_snapshots, 0);
}

#[test]
fn rule_value_mutations_are_transactional_native_and_context_aware() {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let base_url = url::Url::parse("https://example.test/styles/rules.css").unwrap();
    let css_text = concat!(
        "@namespace svg url(\"http://www.w3.org/2000/svg\"); ",
        "@media screen { .subject { color: red; } } ",
        "@font-face { font-family: Before; src: url(before.woff2); } ",
        "@page :first { margin-top: 1px; @top-left { content: \"before\"; } } ",
        "@keyframes fade { from { opacity: 0; } to { opacity: 1; } } ",
        ".host { & .child { color: blue; } color: red; }"
    );
    let first = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let second = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url,
        QuirksMode::NoQuirks,
        shared_lock,
    );
    let second_text_before = serialized_stylesheet_text(&second);

    reset_live_stylesheet_mutation_metrics_for_test();
    let revision = first.contents_revision();
    assert!(matches!(
        first.set_style_rule_selector(&[1, 0], "missing|rect", CssRuleType::Media.bit(), None),
        Err(CssRuleInsertError::Syntax)
    ));
    assert_eq!(first.contents_revision(), revision);
    assert!(ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&second)
    ));

    first
        .set_media_rule_media(&[1], "print and (min-width: 10px)")
        .unwrap();
    first
        .set_style_rule_selector(&[1, 0], "svg|rect", CssRuleType::Media.bit(), None)
        .unwrap();
    first
        .set_style_rule_declarations(&[1, 0], "background-image: url(icon.svg); color: green;")
        .unwrap();
    first
        .set_font_face_rule_descriptors(
            &[2],
            "font-family: After; src: url(after.woff2) !important;",
        )
        .unwrap();
    first
        .set_page_rule_descriptors(&[3], "size: A4; margin-left: 5px; color: red;")
        .unwrap();
    first
        .set_page_margin_rule_descriptors(
            &[3, 0],
            "content: \"after\"; margin-top: 4px; bad-descriptor: 1;",
        )
        .unwrap();
    first
        .set_keyframe_rule_declarations(&[4], 1, "opacity: 0.5; transform: translateX(1px);")
        .unwrap();
    first
        .set_keyframe_rule_selector(&[4], 1, "75%, to")
        .unwrap();
    first
        .set_nested_declarations_rule_declarations(&[5, 1], "padding: 1px 2px;")
        .unwrap();

    let metrics = live_stylesheet_mutation_metrics_for_test();
    assert_eq!(first.contents_revision(), revision + 9);
    assert_eq!(metrics.native_rule_value_mutations, 9);
    assert_eq!(metrics.recursive_rule_snapshots, 0);

    assert_eq!(
        first
            .native_rule_at_path(&[1])
            .unwrap()
            .grouping_prelude(&first.stylesheet.shared_lock)
            .map(|(_, prelude)| prelude)
            .as_deref(),
        Some("print and (min-width: 10px)")
    );
    assert_eq!(
        first
            .native_rule_at_path(&[1, 0])
            .unwrap()
            .style_selector_text(&first.stylesheet.shared_lock)
            .as_deref(),
        Some("svg|rect")
    );
    let first_text = serialized_stylesheet_text(&first);
    assert!(first_text.contains("@media print and (min-width: 10px)"));
    assert!(first_text.contains("svg|rect"));
    assert!(first_text.contains("icon.svg"));
    assert!(first_text.contains("font-family: After"));
    assert!(first_text.contains("after.woff2"));
    assert!(first_text.contains("margin-left: 5px"));
    assert!(first_text.contains("content: \"after\""));
    assert!(!first_text.contains("bad-descriptor"));
    assert!(first_text.contains("75%, 100%"));
    assert!(first_text.contains("padding: 1px 2px"));
    assert_eq!(serialized_stylesheet_text(&second), second_text_before);

    assert_eq!(
        live_stylesheet_mutation_metrics_for_test().recursive_rule_snapshots,
        0
    );
}

#[test]
fn font_feature_values_mutations_preserve_cow_and_wrapper_binding_without_projection() {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let registry = LiveStylesheetRegistry::default();
    let shared_lock = SharedRwLock::new();
    let base_url = url::Url::parse("https://example.test/styles/font-features.css").unwrap();
    let css_text = concat!(
        "@font-feature-values old_family { ",
        "@annotation { first: 1; } ",
        "@styleset { old_set: 2 3; } ",
        "}"
    );
    let first = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url.clone(),
        QuirksMode::NoQuirks,
        shared_lock.clone(),
    );
    let second = registry.create_inline_with_shared_initial_contents(
        css_text,
        base_url,
        QuirksMode::NoQuirks,
        shared_lock,
    );
    let first_id = first.id();
    let original_rule = first.native_rule_at_path(&[0]).unwrap();
    assert!(original_rule.ptr_eq(&second.native_rule_at_path(&[0]).unwrap()));
    let (lease_id, lease) = registry.bind_rule_wrapper(None, &first, vec![0]).unwrap();
    let _lease = lease.unwrap();

    reset_live_stylesheet_mutation_metrics_for_test();
    let revision = first.contents_revision();
    assert!(matches!(
        first.set_font_feature_values_rule_map_entry(
            &[0],
            FontFeatureValuesMapGroup::Annotation,
            "invalid",
            &[1, 2],
        ),
        Err(CssRuleInsertError::Syntax)
    ));
    assert_eq!(first.contents_revision(), revision);
    assert!(ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&second)
    ));

    first
        .set_font_feature_values_rule_font_family(
            &[0],
            vec![
                FamilyName {
                    name: style::Atom::from("serif"),
                    syntax: FontFamilyNameSyntax::Quoted,
                },
                FamilyName {
                    name: style::Atom::from("foo bar"),
                    syntax: FontFamilyNameSyntax::Quoted,
                },
                FamilyName {
                    name: style::Atom::from("changed_family"),
                    syntax: FontFamilyNameSyntax::Identifiers,
                },
            ],
        )
        .unwrap();
    first
        .set_font_feature_values_rule_map_entry(
            &[0],
            FontFeatureValuesMapGroup::Annotation,
            "late",
            &[4],
        )
        .unwrap();
    first
        .set_font_feature_values_rule_map_entry(
            &[0],
            FontFeatureValuesMapGroup::Styleset,
            "old_set",
            &[8, 9],
        )
        .unwrap();
    assert!(
        !first
            .delete_font_feature_values_rule_map_entry(
                &[0],
                FontFeatureValuesMapGroup::Annotation,
                "missing",
            )
            .unwrap()
    );
    assert!(
        first
            .delete_font_feature_values_rule_map_entry(
                &[0],
                FontFeatureValuesMapGroup::Annotation,
                "first",
            )
            .unwrap()
    );
    assert!(
        first
            .clear_font_feature_values_rule_map(&[0], FontFeatureValuesMapGroup::Styleset)
            .unwrap()
    );
    assert!(
        !first
            .clear_font_feature_values_rule_map(&[0], FontFeatureValuesMapGroup::Styleset)
            .unwrap()
    );

    let state = first
        .font_feature_values_rule_at_path(&[0], |rule| {
            (
                rule.family_names.len(),
                rule.annotation
                    .iter()
                    .map(|entry| entry.name.to_string())
                    .collect::<Vec<_>>(),
                rule.styleset.is_empty(),
            )
        })
        .unwrap();
    assert_eq!(state, (3, vec!["late".to_owned()], true));
    assert!(!ServoArc::ptr_eq(
        &stylesheet_contents(&first),
        &stylesheet_contents(&second)
    ));
    let rebound = registry.rule_wrapper_binding(lease_id).unwrap();
    assert_eq!(rebound.stylesheet_id(), Some(first_id));
    assert_eq!(rebound.path, vec![0]);
    assert!(
        rebound
            .rule()
            .ptr_eq(&first.native_rule_at_path(&[0]).unwrap())
    );
    assert!(!rebound.rule().ptr_eq(&original_rule));
    assert!(original_rule.ptr_eq(&second.native_rule_at_path(&[0]).unwrap()));

    let metrics = live_stylesheet_mutation_metrics_for_test();
    assert_eq!(first.contents_revision(), revision + 5);
    assert_eq!(metrics.native_rule_value_mutations, 5);
    assert_eq!(metrics.recursive_rule_snapshots, 0);

    let first_text = serialized_stylesheet_text(&first);
    assert!(first_text.contains("@font-feature-values \"serif\", \"foo bar\", changed_family"));
    assert!(first_text.contains("late: 4"));
    assert!(!first_text.contains("first: 1"));
    assert!(!first_text.contains("@styleset"));
    assert!(serialized_stylesheet_text(&second).contains("@font-feature-values old_family"));
    assert!(serialized_stylesheet_text(&second).contains("first: 1"));
    assert!(serialized_stylesheet_text(&second).contains("old_set: 2 3"));
}

#[test]
fn import_media_mutation_retains_loaded_child_and_wrapper_without_projection() {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let registry = LiveStylesheetRegistry::default();
    let root = registry.create(
        "@import './child.css' layer(theme) supports(display: grid) screen; .root { color: red; }",
        url::Url::parse("https://example.test/styles/root.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    );
    let request = root.pending_import_requests().into_iter().next().unwrap();
    let child = registry
        .install_import_response(
            root.id(),
            root.contents_revision(),
            request.edge_id,
            ".child { color: green; }",
            request.url,
            true,
            true,
        )
        .unwrap();
    let child_stylesheet = child.stylesheet();
    let original_import_rule = root.native_rule_at_path(&[0]).unwrap();
    let (lease_id, lease) = registry.bind_rule_wrapper(None, &root, vec![0]).unwrap();
    let _lease = lease.unwrap();

    reset_live_stylesheet_mutation_metrics_for_test();
    let revision = root.contents_revision();
    root.set_import_rule_media(&[0], "print and (width)")
        .unwrap();

    let imported_stylesheet = match root.native_rule_at_path(&[0]).unwrap() {
        NativeStylesheetRule::Css(CssRule::Import(rule)) => {
            let guard = root.stylesheet.shared_lock.read();
            rule.read_with(&guard)
                .stylesheet
                .as_sheet()
                .cloned()
                .unwrap()
        }
        _ => unreachable!("expected import rule"),
    };
    assert!(ServoArc::ptr_eq(&imported_stylesheet, &child_stylesheet));
    let media_text = {
        let guard = imported_stylesheet.shared_lock.read();
        imported_stylesheet.media.read_with(&guard).to_css_string()
    };
    assert_eq!(media_text, "print and (width)");

    let binding = registry.rule_wrapper_binding(lease_id).unwrap();
    assert_eq!(binding.stylesheet_id(), Some(root.id()));
    assert_eq!(binding.path, vec![0]);
    assert!(binding.rule().ptr_eq(&original_import_rule));
    let metrics = live_stylesheet_mutation_metrics_for_test();
    assert_eq!(root.contents_revision(), revision + 1);
    assert_eq!(metrics.native_rule_value_mutations, 1);
    assert_eq!(metrics.recursive_rule_snapshots, 0);

    let rule_text = native_rule_css_text(&root, &[0]);
    assert!(rule_text.contains("url(\"./child.css\")"));
    assert!(rule_text.contains("layer(theme)"));
    assert!(rule_text.contains("supports(display: grid)"));
    assert!(rule_text.contains("print and (width)"));
}

fn stylesheet_with_rule_count(
    registry: &LiveStylesheetRegistry,
    rule_count: usize,
    nested: bool,
) -> LiveStylesheetRef {
    let mut css_text = String::with_capacity(rule_count.saturating_mul(32) + 16);
    if nested {
        css_text.push_str("@media all {");
    }
    for index in 0..rule_count {
        use std::fmt::Write;
        write!(css_text, ".rule-{index}{{--index:{index}}}")
            .expect("writing CSS into a String must not fail");
    }
    if nested {
        css_text.push('}');
    }
    registry.create(
        &css_text,
        url::Url::parse("https://path-scaling.test/styles.css").unwrap(),
        QuirksMode::NoQuirks,
        AllowImportRules::Yes,
        SharedRwLock::new(),
    )
}

fn measure_rule_wrapper_path_updates(
    rule_count: usize,
    materialized_count: usize,
    rounds: usize,
    nested: bool,
) -> std::time::Duration {
    let registry = LiveStylesheetRegistry::default();
    let stylesheet = stylesheet_with_rule_count(&registry, rule_count, nested);
    let child_indexes = if materialized_count == 3 {
        vec![0, rule_count / 2, rule_count - 1]
    } else {
        (0..materialized_count).collect()
    };
    let parent_path = if nested { &[0][..] } else { &[][..] };
    let paths = child_indexes
        .into_iter()
        .map(|index| child_rule_path(parent_path, index))
        .collect::<Vec<_>>();
    let mut leases = Vec::with_capacity(paths.len());
    let mut lease_ids = Vec::with_capacity(paths.len());
    for path in &paths {
        let (lease_id, lease) = registry
            .bind_rule_wrapper(None, &stylesheet, path.clone())
            .expect("benchmark rule path should exist");
        lease_ids.push(lease_id);
        leases.push(lease.expect("new rule binding should return its lease"));
    }
    assert_eq!(stylesheet.rule_wrapper_leases.borrow().len(), paths.len());

    let middle = rule_count / 2;
    let start = std::time::Instant::now();
    for _ in 0..rounds {
        stylesheet.shift_rule_wrapper_paths_for_insert(parent_path, middle);
        stylesheet.shift_rule_wrapper_paths_for_delete(parent_path, middle);
    }
    let elapsed = start.elapsed();

    for (lease_id, expected_path) in lease_ids.into_iter().zip(paths) {
        assert_eq!(
            registry.rule_wrapper_binding(lease_id).unwrap().path,
            expected_path
        );
    }
    std::hint::black_box(leases);
    elapsed
}

#[test]
#[ignore = "manual release-only CSSOM path scaling evidence"]
fn rule_wrapper_path_scaling_evidence() {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    for nested in [false, true] {
        for rule_count in [1_000, 10_000] {
            for materialized_count in [3, rule_count] {
                let rounds = if materialized_count == 3 { 10_000 } else { 100 };
                let elapsed = measure_rule_wrapper_path_updates(
                    rule_count,
                    materialized_count,
                    rounds,
                    nested,
                );
                println!(
                    "scope={} rules={rule_count} materialized={materialized_count} rounds={rounds} elapsed_us={} ns_per_round={}",
                    if nested { "nested" } else { "top-level" },
                    elapsed.as_micros(),
                    elapsed.as_nanos() / rounds as u128,
                );
            }
        }
    }
}
