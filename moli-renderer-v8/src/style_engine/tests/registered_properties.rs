use super::*;

fn test_url() -> url::Url {
    url::Url::parse("https://example.test/").unwrap()
}

fn register_custom_property(
    engine: &mut MoliStyleEngine,
    document: DomHandle,
    name: &str,
    syntax: &str,
    initial_value: &str,
) {
    engine
        .register_css_custom_property_for_document(
            document,
            CssCustomPropertyRegistration {
                name: name.to_owned(),
                syntax: syntax.to_owned(),
                inherits: false,
                initial_value: Some(initial_value.to_owned()),
            },
            test_url(),
        )
        .expect("custom property registration should succeed");
}

fn registration(name: &str, syntax: &str, initial_value: &str) -> CssCustomPropertyRegistration {
    CssCustomPropertyRegistration {
        name: name.to_owned(),
        syntax: syntax.to_owned(),
        inherits: false,
        initial_value: Some(initial_value.to_owned()),
    }
}

fn computed_value(
    engine: &MoliStyleEngine,
    host: &DomHost,
    target: DomHandle,
    property: &str,
    document_sources: &[StyloStylesheetSource],
) -> String {
    let mut inputs = FullStyleWorldSnapshot::default();
    inputs
        .document_stylesheet_sources
        .extend(document_sources.iter().cloned());
    let document = host
        .owner_document_handle(target)
        .unwrap_or_else(|| host.document_handle());
    inputs.script_custom_property_registrations =
        engine.script_css_custom_property_registration_records_for_document(document);
    engine
        .computed_style_property_value(host, &test_url(), target, property, None, &inputs, None)
        .unwrap_or_default()
}

#[test]
fn css_register_property_does_not_advance_source_set_generation() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.append_child(document, target));
    let mut engine = MoliStyleEngine::new();
    assert_eq!(
        computed_value(&engine, &host, target, "display", &[]),
        "block"
    );
    let source_set_generation = engine.source_set_generation_for_document_for_test(document);
    let computed_generation = engine.computed_cache_generation_for_document_for_test(document);
    let stylist_identity = engine.retained_stylist_identity_for_document_for_test(document);
    let rebuilds = engine.retained_style_system_rebuild_count_for_document_for_test(document);
    let updates = engine.retained_style_system_update_count_for_document_for_test(document);

    register_custom_property(
        &mut engine,
        document,
        "--registered-color",
        "<color>",
        "rgb(1, 2, 3)",
    );

    assert_eq!(
        engine.source_set_generation_for_document_for_test(document),
        source_set_generation,
        "CSS.registerProperty changes author style semantics but not the stylesheet source set"
    );
    assert_eq!(
        engine.computed_cache_generation_for_document_for_test(document),
        computed_generation,
        "CSS.registerProperty only marks the document dirty until the next observation"
    );
    assert_eq!(
        engine.source_dirty_scope_reasons_for_document_for_test(document),
        vec![StyleSourceDirtyReason::CustomPropertyRegistration]
    );

    assert_eq!(
        computed_value(&engine, &host, target, "--registered-color", &[]),
        "rgb(1, 2, 3)"
    );
    assert_eq!(
        engine.retained_stylist_identity_for_document_for_test(document),
        stylist_identity,
        "registerProperty must update the existing document Stylist"
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
            .is_empty(),
        "the observation must consume the registration dirty marker"
    );
}

#[test]
fn registered_custom_properties_keep_their_registration_time_base_urls() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.append_child(document, target));
    let mut engine = MoliStyleEngine::new();

    for (name, base_url) in [
        ("--first-image", "https://example.test/first/"),
        ("--second-image", "https://example.test/second/"),
    ] {
        engine
            .register_css_custom_property_for_document(
                document,
                registration(name, "<image>", "url(icon.svg)"),
                url::Url::parse(base_url).unwrap(),
            )
            .expect("custom property registration should succeed");
    }

    assert_eq!(
        computed_value(&engine, &host, target, "--first-image", &[]),
        r#"url("https://example.test/first/icon.svg")"#
    );
    assert_eq!(
        computed_value(&engine, &host, target, "--second-image", &[]),
        r#"url("https://example.test/second/icon.svg")"#
    );
}

#[test]
fn invalid_registered_property_value_substitutes_registered_initial_value() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    register_custom_property(
        &mut engine,
        document,
        "--registered-length",
        "<length>",
        "0px",
    );
    let sources = vec![StyloStylesheetSource::new(
        "#target {
            --registered-length: red;
            --unregistered: var(--registered-length);
        }"
        .into(),
        test_url(),
    )];

    assert_eq!(
        computed_value(&engine, &host, target, "--registered-length", &sources),
        "0px"
    );
    assert_eq!(
        computed_value(&engine, &host, target, "--unregistered", &sources),
        "0px"
    );
}

#[test]
fn custom_property_registry_is_scoped_to_document_world() {
    let mut host = test_host();
    let active_document = host.document_handle();
    let detached_document = host.create_detached_html_document();
    let mut engine = MoliStyleEngine::new();

    engine
        .register_css_custom_property_for_document(
            active_document,
            registration("--active-only", "<length>", "1px"),
            test_url(),
        )
        .expect("active document registration should succeed");
    engine
        .register_css_custom_property_for_document(
            detached_document,
            registration("--detached-only", "<color>", "red"),
            test_url(),
        )
        .expect("detached document registration should succeed");

    assert!(
        engine
            .registered_css_custom_property_registration_for_document(
                active_document,
                "--active-only"
            )
            .is_some()
    );
    assert!(
        engine
            .registered_css_custom_property_registration_for_document(
                active_document,
                "--detached-only"
            )
            .is_none()
    );
    assert!(
        engine
            .registered_css_custom_property_registration_for_document(
                detached_document,
                "--detached-only"
            )
            .is_some()
    );
    assert!(
        engine
            .registered_css_custom_property_registration_for_document(
                detached_document,
                "--active-only"
            )
            .is_none()
    );
}

#[test]
fn registered_invalid_color_value_substitutes_initial_value_into_normal_property() {
    let mut host = test_host();
    let document = host.document_handle();
    let outer = host.create_element("div");
    let target = host.create_element("div");
    assert!(host.set_attribute(outer, "id", "outer"));
    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.append_child(document, outer));
    assert!(host.append_child(outer, target));

    let mut engine = MoliStyleEngine::new();
    register_custom_property(
        &mut engine,
        document,
        "--registered-color",
        "<color>",
        "rgb(3, 3, 3)",
    );
    let sources = vec![StyloStylesheetSource::new(
        "#outer {
            color: rgb(1, 1, 1);
        }
        #target {
            --registered-color: rgb(2, 2, 2);
            --registered-color: url(not-a-color);
            color: var(--registered-color);
        }"
        .into(),
        test_url(),
    )];

    assert_eq!(
        computed_value(&engine, &host, target, "color", &sources),
        "rgb(3, 3, 3)"
    );
}

#[test]
fn registered_length_lh_units_resolve_after_line_height() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    register_custom_property(&mut engine, document, "--registered-lh", "<length>", "0px");
    let sources = vec![StyloStylesheetSource::new(
        "#target {
            font-size: 10px;
            line-height: 20px;
            --registered-lh: 10lh;
        }"
        .into(),
        test_url(),
    )];

    assert_eq!(
        computed_value(&engine, &host, target, "line-height", &sources),
        "20px"
    );
    assert_eq!(
        computed_value(&engine, &host, target, "--registered-lh", &sources),
        "200px"
    );
}

#[test]
fn registered_color_values_resolve_currentcolor() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    register_custom_property(
        &mut engine,
        document,
        "--registered-color",
        "<color>",
        "transparent",
    );
    let sources = vec![StyloStylesheetSource::new(
        "#target {
            color: blue;
            --registered-color: currentcolor;
        }"
        .into(),
        test_url(),
    )];

    assert_eq!(
        computed_value(&engine, &host, target, "color", &sources),
        "rgb(0, 0, 255)"
    );
    assert_eq!(
        computed_value(&engine, &host, target, "--registered-color", &sources),
        "rgb(0, 0, 255)"
    );
}

#[test]
fn registered_color_functions_resolve_currentcolor() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    register_custom_property(
        &mut engine,
        document,
        "--registered-color",
        "<color>",
        "transparent",
    );
    let sources = vec![StyloStylesheetSource::new(
        "#target {
            color: blue;
            --registered-color: color-mix(in srgb, currentcolor, red);
        }"
        .into(),
        test_url(),
    )];

    assert_eq!(
        computed_value(&engine, &host, target, "--registered-color", &sources),
        "color(srgb 0.5 0 0.5)"
    );
}

#[test]
fn registered_relative_colors_resolve_currentcolor() {
    let mut host = test_host();
    let document = host.document_handle();
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.append_child(document, target));

    let mut engine = MoliStyleEngine::new();
    register_custom_property(
        &mut engine,
        document,
        "--registered-color",
        "<color>",
        "transparent",
    );
    let sources = vec![StyloStylesheetSource::new(
        "#target {
            color: blue;
            --registered-color: color(from currentcolor srgb b g r);
        }"
        .into(),
        test_url(),
    )];

    assert_eq!(
        computed_value(&engine, &host, target, "--registered-color", &sources),
        "color(srgb 1 0 0)"
    );
}

#[test]
fn registered_color_values_resolve_tree_counting_math() {
    let mut host = test_host();
    let document = host.document_handle();
    let parent = host.create_element("div");
    let target = host.create_element("div");
    assert!(host.set_attribute(target, "id", "target"));
    assert!(host.append_child(document, parent));
    assert!(host.append_child(parent, target));

    let mut engine = MoliStyleEngine::new();
    register_custom_property(
        &mut engine,
        document,
        "--registered-color",
        "<color>",
        "black",
    );
    let sources = vec![StyloStylesheetSource::new(
        "#target {
            --registered-color: color(srgb 0 sibling-index() 0);
        }"
        .into(),
        test_url(),
    )];

    assert_eq!(
        computed_value(&engine, &host, target, "--registered-color", &sources),
        "color(srgb 0 1 0)"
    );
}
