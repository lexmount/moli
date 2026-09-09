use super::*;

pub(in crate::native_bridge::element::styles) fn style_property_priority(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> String {
    let Some(property) = canonical_specified_cssom_query_property_name(property) else {
        return String::new();
    };
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(priority) = inline_state_property_priority_with_pdb(state, &property)
    {
        return if priority {
            "important".to_owned()
        } else {
            String::new()
        };
    }
    let entries = style_entries(runtime, handle);
    if let Some(priority) = style_entries_property_priority_with_pdb(&entries, &property) {
        return if priority {
            "important".to_owned()
        } else {
            String::new()
        };
    }
    if let Some(entry) = inline_style_entry(runtime, handle, &property) {
        return if entry.priority {
            "important".to_owned()
        } else {
            String::new()
        };
    }
    if let Some(longhands) = shorthand_longhands(&property) {
        let mut priority = None;
        for longhand in longhands {
            let Some(entry) = inline_style_entry(runtime, handle, longhand) else {
                return String::new();
            };
            if priority.is_some_and(|current| current != entry.priority) {
                return String::new();
            }
            priority = Some(entry.priority);
        }
        if priority == Some(true) {
            return "important".to_owned();
        }
    }
    String::new()
}

pub(super) fn canonical_specified_cssom_query_property_name(property: &str) -> Option<String> {
    if property.starts_with("--") && !moli_css_parse::is_cssom_custom_property_name(property) {
        return None;
    }
    let property = canonical_style_property_name(property);
    if !property.starts_with("--") && !known_style_property(&property) {
        return None;
    }
    Some(if property == "-webkit-transform" {
        "transform".to_owned()
    } else {
        property
    })
}

pub(super) fn canonical_computed_cssom_query_property_name(property: &str) -> Option<String> {
    if property.starts_with("--") {
        return moli_css_parse::is_cssom_custom_property_name(property)
            .then(|| property.to_owned());
    }
    let property = canonical_style_property_name(property);
    let property = if property == "-webkit-transform" {
        "transform".to_owned()
    } else {
        property
    };
    computed_property_is_queryable(&property).then_some(property)
}

pub(in crate::native_bridge::element::styles) fn style_css_text_for_computed(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> String {
    let _ = (runtime, handle);
    String::new()
}

pub(super) fn specified_color_value_is_valid(value: &str) -> bool {
    let value = value.trim();
    if moli_css_parse::css_value_may_contain_env_function(value) {
        return moli_css_parse::css_declaration_value_has_valid_env_functions(value);
    }
    matches!(
        value.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "revert-rule"
    ) || specified_color_component_value_is_valid(value)
}

fn specified_color_component_value_is_valid(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("transparent")
        || value.eq_ignore_ascii_case("currentcolor")
        || ident_is_system_color(value)
        || css_named_color_rgb(value).is_some()
        || css_hex_color_rgb(value).is_some()
        || ((value.starts_with("rgb(") || value.starts_with("rgba(")) && value.ends_with(')'))
}

pub(in crate::native_bridge::element::styles) fn style_property_names_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    context: StyleComputationContext,
) -> Vec<String> {
    if mode == StyleMode::Computed {
        return super::super::super::computed_names::computed_property_names(
            runtime, handle, context,
        );
    }
    if let Some(state) = runtime.element_inline_style_declaration_state(handle) {
        return state.property_names();
    }
    style_entries(runtime, handle)
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

pub(in crate::native_bridge::element::styles) fn style_property_count_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    context: StyleComputationContext,
) -> usize {
    if mode == StyleMode::Computed {
        return super::super::super::computed_names::computed_property_count(
            runtime, handle, context,
        );
    }
    style_property_names_with_context(runtime, handle, mode, context).len()
}

pub(in crate::native_bridge::element::styles) fn style_property_name_at_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    context: StyleComputationContext,
    index: usize,
) -> Option<String> {
    if mode == StyleMode::Computed {
        return super::super::super::computed_names::computed_property_name_at(
            runtime, handle, context, index,
        );
    }
    style_property_names_with_context(runtime, handle, mode, context)
        .get(index)
        .cloned()
}

pub(in crate::native_bridge::element::styles) fn style_property_index_exists_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    context: StyleComputationContext,
    index: usize,
) -> bool {
    if mode == StyleMode::Computed {
        return super::super::super::computed_names::computed_property_name_at(
            runtime, handle, context, index,
        )
        .is_some();
    }
    index < style_property_count_with_context(runtime, handle, mode, context)
}

pub(in crate::native_bridge::element::styles) fn computed_style_applies(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    runtime
        .dom_host()
        .containing_shadow_root(handle)
        .is_none_or(|shadow_root| !runtime.shadow_root_is_disconnected_for_style(shadow_root))
}

#[cfg(test)]
mod tests {
    use super::super::super::style_world::connected_shadow_roots_for_test;
    use super::{
        KEYFRAME_NESTING_DEPTH_LIMIT, animation_shorthand_names, box_shorthand_component,
        collect_custom_functions_from_css, compress_box_shorthand_value,
        custom_function_container_rule_texts, format_css_number, format_css_px,
        keyframe_has_supported_animation_values, keyframe_property_values,
        normalize_computed_color_functions, normalize_css_integer_token, normalize_style_value,
        simple_var_function_parts,
    };
    use crate::dom::native::{DomHost, NativeDom};
    use crate::native_bridge::DomHandle;
    use std::collections::HashMap;

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ))
    }

    fn connect_for_test(host: &mut DomHost, parent: DomHandle, child: DomHandle) {
        assert!(host.append_child_without_mutation_effects(parent, child));
    }

    fn nested_keyframes(depth: usize) -> String {
        let mut css = "@keyframes anim { from { left: 0px; } to { left: 10px; } }".to_owned();
        for _ in 0..depth {
            css = format!("@media all {{ {css} }}");
        }
        css
    }

    #[test]
    fn css_number_serialization_discards_bounded_f32_integer_noise() {
        assert_eq!(format_css_number(120.000005), "120");
        assert_eq!(format_css_number(-120.000005), "-120");
        assert_eq!(format_css_number(120.00005), "120.00005");
    }

    #[test]
    fn css_pixel_serialization_matches_blink_six_significant_digits() {
        assert_eq!(format_css_px(33.328_125), "33.3281px");
        assert_eq!(format_css_px(0.015_625), "0.015625px");
        assert_eq!(format_css_px(999_999.0), "999999px");
        assert_eq!(format_css_px(1_000_000.0), "1e+06px");
    }

    #[test]
    fn connected_shadow_roots_for_document_excludes_child_document_roots() {
        let mut host = test_host();
        let document = host.document_handle();
        let active_host = host.create_element("section");
        connect_for_test(&mut host, document, active_host);
        let active_root = host
            .attach_shadow_root(active_host, "open")
            .expect("active document host should accept shadow root");

        let child_document = host.create_detached_html_document();
        let child_host = host.create_parser_element_without_attributes_for_document(
            child_document,
            "article".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        connect_for_test(&mut host, child_document, child_host);
        let child_root = host
            .attach_shadow_root(child_host, "open")
            .expect("child document host should accept shadow root");
        host.mark_subtree_connected_preserving_owner_document(child_document);

        assert_eq!(
            connected_shadow_roots_for_test(&host, document),
            vec![active_root]
        );
        assert_eq!(
            connected_shadow_roots_for_test(&host, child_document),
            vec![child_root]
        );
    }

    #[test]
    fn box_shorthand_component_keeps_function_whitespace_internal() {
        assert_eq!(
            box_shorthand_component("calc(10px + 5px) auto", 0).as_deref(),
            Some("calc(10px + 5px)")
        );
        assert_eq!(
            box_shorthand_component("calc(10px + 5px) auto", 1).as_deref(),
            Some("auto")
        );
        assert_eq!(
            box_shorthand_component("calc(50% + 2px) 4px", 0).as_deref(),
            Some("calc(50% + 2px)")
        );
        assert_eq!(
            box_shorthand_component("4px calc(50% + 2px)", 1).as_deref(),
            Some("calc(50% + 2px)")
        );
    }

    #[test]
    fn font_shorthand_slash_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_style_value("font", "10px/1 Ahem"),
            "10px / 1 Ahem"
        );
        assert_eq!(
            normalize_style_value("font", "var(--font/size) / var(--line/height) Ahem"),
            "var(--font/size) / var(--line/height) Ahem"
        );
    }

    #[test]
    fn content_value_normalization_stays_renderer_local() {
        assert_eq!(normalize_style_value("content", "'string'"), r#""string""#);
        assert_eq!(
            normalize_style_value("content", "url(http://localhost/)"),
            r#"url("http://localhost/")"#
        );
        assert_eq!(
            normalize_style_value("content", "counter(par-num, decimal)"),
            "counter(par-num)"
        );
        assert_eq!(
            normalize_style_value("content", "attr( |bar )"),
            "attr( |bar )"
        );
    }

    #[test]
    fn font_family_value_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_style_value("font-family", "'Lucida Grande'"),
            r#""Lucida Grande""#
        );
        assert_eq!(normalize_style_value("font-family", "'Arial'"), "Arial");
        assert_eq!(normalize_style_value("font-family", "'-Arial'"), "-Arial");
        assert_eq!(normalize_style_value("font-family", "'-'"), r#""-""#);
        assert_eq!(normalize_style_value("font-family", "'--'"), r#""--""#);
        assert_eq!(normalize_style_value("font-family", "'-1'"), r#""-1""#);
        assert_eq!(normalize_style_value("font-family", "'34J'"), r#""34J""#);
        assert_eq!(
            normalize_style_value("font-family", "'serif'"),
            r#""serif""#
        );
        assert_eq!(normalize_style_value("font-family", "'A  B'"), r#""A  B""#);
    }

    #[test]
    fn css_integer_token_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_css_integer_token("1111111111111111111111111").as_deref(),
            Some("1111111111111111111111111")
        );
        assert_eq!(normalize_css_integer_token("+0012").as_deref(), Some("12"));
        assert_eq!(normalize_css_integer_token("-000").as_deref(), Some("0"));
        assert_eq!(normalize_css_integer_token("1.0"), None);
        assert_eq!(normalize_css_integer_token("1px"), None);
    }

    #[test]
    fn simple_var_function_projection_stays_renderer_local() {
        assert_eq!(
            simple_var_function_parts("var(--x)")
                .filter(|parts| parts.fallback.is_none())
                .map(|parts| parts.name)
                .as_deref(),
            Some("--x")
        );
        assert_eq!(
            simple_var_function_parts("var(--x, red)")
                .and_then(|parts| parts.fallback)
                .as_deref(),
            Some("red")
        );
        assert_eq!(
            simple_var_function_parts(" var( --x ) ")
                .filter(|parts| parts.fallback.is_none())
                .map(|parts| parts.name)
                .as_deref(),
            Some("--x")
        );
        assert!(
            simple_var_function_parts("var(--x, 1)").is_some_and(|parts| parts.fallback.is_some())
        );
        assert_eq!(simple_var_function_parts("var(x)"), None);
        assert_eq!(simple_var_function_parts("calc(var(--x))"), None);

        assert_eq!(normalize_style_value("color", "var(--x)"), "var(--x)");
        assert_eq!(normalize_style_value("z-index", "var(--z)"), "var(--z)");
    }

    #[test]
    fn animation_shorthand_name_projection_stays_renderer_local() {
        assert_eq!(
            animation_shorthand_names("1s linear infinite alternate anim"),
            vec!["anim"]
        );
        assert_eq!(
            animation_shorthand_names("spin 1s ease, 200ms fade-in forwards"),
            vec!["spin", "fade-in"]
        );
        assert!(animation_shorthand_names("none").is_empty());
    }

    #[test]
    fn anchor_size_normalization_stays_renderer_local() {
        assert_eq!(
            normalize_style_value(
                "width",
                "anchor-size(width, anchor-size(--foo height, 10px))"
            ),
            "anchor-size(width, anchor-size(--foo height, 10px))"
        );
        assert_eq!(
            normalize_style_value("width", "anchor-size(width --target)"),
            "anchor-size(--target width)"
        );
    }

    #[test]
    fn custom_css_function_parser_extracts_container_results() {
        let mut functions = HashMap::new();
        collect_custom_functions_from_css(
            r#"
            @function --b() {
              @container --cont (width = 5px) { result: 5px; }
              @container --cont (width = 10px) { result: 10px; }
            }
            "#,
            &mut functions,
        );
        let function = functions.get("--b").expect("custom function");
        assert_eq!(function.container_results.len(), 2);
        assert_eq!(function.container_results[0].container_name, "--cont");
        assert_eq!(function.container_results[0].width_px, 5.0);
        assert_eq!(function.container_results[0].result, "5px");
    }

    #[test]
    fn custom_css_function_container_projection_uses_rule_local_css_text() {
        let rules = custom_function_container_rule_texts(
            r#"
            @container --cont (width = 5px) { result: 5px; }
            @container --next (width = 10px) { result: 10px; }
            "#,
        );
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[0].css_text,
            "@container --cont (width = 5px) {result: 5px;}"
        );
        assert!(
            !rules[0].css_text.contains("--next"),
            "custom-function container projection must not capture trailing rules"
        );
    }

    #[test]
    fn compress_box_shorthand_value_keeps_function_components_intact() {
        assert_eq!(
            compress_box_shorthand_value("calc(5px + 5px) 1px calc(5px + 5px) 1px"),
            "calc(5px + 5px) 1px"
        );
        assert_eq!(
            compress_box_shorthand_value(
                "calc(5px + 5px) calc(5px + 5px) calc(5px + 5px) calc(5px + 5px)"
            ),
            "calc(5px + 5px)"
        );
    }

    #[test]
    fn normalize_computed_color_functions_replaces_only_full_system_color_tokens() {
        assert_eq!(
            normalize_computed_color_functions("1px 1px MenuText", None),
            "rgb(0, 0, 0) 1px 1px"
        );
        assert_eq!(
            normalize_computed_color_functions("1px 1px menutext", None),
            "rgb(0, 0, 0) 1px 1px"
        );
        assert_eq!(
            normalize_computed_color_functions("1px 1px NotMenuText", None),
            "1px 1px NotMenuText"
        );
        assert_eq!(
            normalize_computed_color_functions("1px 1px menutext, 2px 2px linktext", None),
            "rgb(0, 0, 0) 1px 1px, rgb(0, 0, 238) 2px 2px"
        );
        assert_eq!(
            normalize_computed_color_functions(
                "1px 1px color-mix(in srgb, rgb(0,0,0), white)",
                None
            ),
            "1px 1px color-mix(in srgb, rgb(0,0,0), white)"
        );
        assert_eq!(
            normalize_computed_color_functions(
                "1px 1px currentcolor, 2px 2px LinkText",
                Some("rgb(10, 20, 30)")
            ),
            "rgb(10, 20, 30) 1px 1px, rgb(0, 0, 238) 2px 2px"
        );
    }

    #[test]
    fn keyframe_supported_animation_scan_has_depth_limit() {
        let names = vec!["anim".to_owned()];

        assert!(keyframe_has_supported_animation_values(
            &nested_keyframes(KEYFRAME_NESTING_DEPTH_LIMIT),
            &names
        ));
        assert!(!keyframe_has_supported_animation_values(
            &nested_keyframes(KEYFRAME_NESTING_DEPTH_LIMIT + 1),
            &names
        ));
    }

    #[test]
    fn keyframe_animation_scan_uses_stylo_nested_rule_snapshots() {
        let names = vec!["move".to_owned()];
        let css = r#"
            @media all {
              @supports (display: block) {
                @keyframes move {
                  from { left: 0px; }
                  to { left: 20px; }
                }
              }
            }
        "#;

        assert!(keyframe_has_supported_animation_values(css, &names));
        assert_eq!(
            keyframe_property_values(css, &names, "left"),
            Some(("0px".to_owned(), "20px".to_owned()))
        );
    }

    #[test]
    fn keyframe_animation_scan_uses_pdb_declaration_values() {
        let names = vec!["fade".to_owned()];
        let css = r#"
            @keyframes fade {
              from {
                background-color: rgb(0 128 0 / 50%);
                width: calc(7px * up);
              }
              to {
                background-color: rgb(0 128 0 / 50%);
                width: calc(10px + 1vmin + 10%);
              }
            }
        "#;

        assert!(keyframe_has_supported_animation_values(css, &names));
        assert_eq!(
            keyframe_property_values(css, &names, "background-color"),
            Some((
                "rgba(0, 128, 0, 0.5)".to_owned(),
                "rgba(0, 128, 0, 0.5)".to_owned()
            ))
        );
        assert_eq!(keyframe_property_values(css, &names, "width"), None);
    }
}
