use crate::css_style::{CssInlineStyleDeclarationState, CssStyleEntry as StyleEntry};
use crate::native_bridge::element::styles::declaration::properties::known_style_property;

use super::cssom_mutation::parse_style_property_entries_for_cssom_write;
use super::declaration_parser::parse_style_property_entries_with_base;
use super::inline_state::{
    expand_unresolved_box_shorthand_entries_for_mutation, inline_css_text_pdb_storage_state,
    inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse,
    inline_state_block_entries_for_property_mutation,
    inline_state_has_replaceable_side_entries_for_property,
    inline_state_has_unpreservable_side_entries_for_property,
    inline_state_property_priority_with_pdb, inline_state_property_value_with_pdb,
    inline_style_declaration_state_from_css_text, inline_style_declaration_state_from_entries,
    inline_style_declaration_state_from_serialized_entries,
    refresh_inline_state_entries_after_pdb_mutation, unresolved_box_shorthand_longhands,
};
use super::pdb_compat::{
    cssom_style_property_write_uses_pdb, inline_style_entry_is_pdb_storage_candidate,
    parse_style_property_entries_with_pdb, set_pdb_block_property_collecting_entries,
    style_entry_is_pdb_safe, style_entry_is_pdb_supplemental_side_entry,
    style_property_affected_names_with_pdb, style_property_mutation_affected_names_with_pdb,
    stylo_pdb_entries_for_property,
};
use super::property_access::{
    pdb_property_priority_for_cssom_query_with_side_entries,
    pdb_property_value_for_cssom_query_with_side_entries,
};
use super::{
    animation_shorthand_longhands, font_shorthand_longhands, font_variant_longhands,
    transition_shorthand_longhands,
};

fn style_entry(name: &str, value: &str) -> StyleEntry {
    StyleEntry {
        name: name.to_owned(),
        value: value.to_owned(),
        priority: false,
    }
}

fn important_style_entry(name: &str, value: &str) -> StyleEntry {
    StyleEntry {
        name: name.to_owned(),
        value: value.to_owned(),
        priority: true,
    }
}

#[test]
fn css_math_properties_reject_invalid_values_with_stylo_parser() {
    for (property, value) in [
        ("transform", "rotate(calc((0.25turn error)))"),
        ("width", "calc(7px * up)"),
        ("width", "calc(5px / 1px)"),
        ("width", "calc(5px * 1px)"),
        ("width", "round(nearest, 1px, 1px, 1px)"),
        ("width", "round(nearest, 1px)"),
        ("width", "calc([])"),
        ("width", "calc( [])"),
    ] {
        assert!(
            parse_style_property_entries_with_base(property, value, false, None).is_none(),
            "{property}: {value} should be rejected"
        );
    }
}

#[test]
fn cssom_write_rejects_mixed_css_wide_keywords() {
    for (property, value) in [
        ("border-spacing", "5px inherit"),
        ("margin", "inherit 5px"),
        ("border-radius", "1px 0 3px inherit"),
        ("overflow", "inherit scroll"),
    ] {
        assert!(
            parse_style_property_entries_for_cssom_write(property, value, false, None).is_none(),
            "{property}: {value} should reject CSS-wide keyword mixed with ordinary values"
        );
    }

    assert!(
        parse_style_property_entries_for_cssom_write("border-spacing", "inherit", false, None)
            .is_some()
    );
}

#[test]
fn css_math_properties_serialize_through_stylo_parser() {
    let cssom_width = parse_style_property_entries_for_cssom_write(
        "width",
        "calc(10px + 1vmin + 10%)",
        false,
        None,
    )
    .expect("CSSOM width should parse through the PDB value-fragment path");
    assert!(
        cssom_style_property_write_uses_pdb("width", "calc(10px + 1vmin + 10%)"),
        "numeric longhand CSSOM writes should no longer fall back to the renderer base parser"
    );
    assert_eq!(cssom_width.entries.len(), 1);
    assert_eq!(cssom_width.entries[0].name, "width");
    assert_eq!(cssom_width.entries[0].value, "calc(10% + 10px + 1vmin)");
    assert!(style_entry_is_pdb_safe(&cssom_width.entries[0]));

    let cssom_height =
        parse_style_property_entries_for_cssom_write("height", "clamp(1px,2px,3px)", false, None)
            .expect("CSSOM height should parse through the PDB value-fragment path");
    assert!(cssom_style_property_write_uses_pdb(
        "height",
        "clamp(1px,2px,3px)"
    ));
    assert_eq!(cssom_height.entries.len(), 1);
    assert_eq!(cssom_height.entries[0].name, "height");
    assert_eq!(cssom_height.entries[0].value, "calc(2px)");
    assert!(style_entry_is_pdb_safe(&cssom_height.entries[0]));

    let cssom_margin =
        parse_style_property_entries_for_cssom_write("margin", "1px 2px", false, None)
            .expect("CSSOM margin shorthand should parse through the PDB value-fragment path");
    assert!(
        cssom_style_property_write_uses_pdb("margin", "1px 2px"),
        "physical box shorthand CSSOM writes should no longer fall back to the entries adapter"
    );
    assert_eq!(
        cssom_margin
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [
            ("margin-top", "1px"),
            ("margin-right", "2px"),
            ("margin-bottom", "1px"),
            ("margin-left", "2px")
        ]
    );
    assert!(cssom_margin.entries.iter().all(style_entry_is_pdb_safe));

    let cssom_padding = parse_style_property_entries_for_cssom_write(
        "padding",
        "calc(calc(12px)) 2px",
        false,
        None,
    )
    .expect("CSSOM padding shorthand should parse through the PDB value-fragment path");
    assert!(
        cssom_style_property_write_uses_pdb("padding", "calc(calc(12px)) 2px"),
        "physical padding shorthand CSSOM writes should no longer fall back to the entries adapter"
    );
    assert_eq!(
        cssom_padding
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [
            ("padding-top", "calc(12px)"),
            ("padding-right", "2px"),
            ("padding-bottom", "calc(12px)"),
            ("padding-left", "2px")
        ]
    );
    assert!(cssom_padding.entries.iter().all(style_entry_is_pdb_safe));

    let cssom_margin_top = parse_style_property_entries_for_cssom_write(
        "margin-top",
        "clamp(1px,2px,3px)",
        false,
        None,
    )
    .expect("CSSOM margin-top should parse through the PDB value-fragment path");
    assert!(
        cssom_style_property_write_uses_pdb("margin-top", "clamp(1px,2px,3px)"),
        "physical box longhand CSSOM writes should no longer fall back to the entries adapter"
    );
    assert_eq!(cssom_margin_top.entries.len(), 1);
    assert_eq!(cssom_margin_top.entries[0].name, "margin-top");
    assert_eq!(cssom_margin_top.entries[0].value, "calc(2px)");
    assert!(style_entry_is_pdb_safe(&cssom_margin_top.entries[0]));

    let cssom_margin_block =
        parse_style_property_entries_for_cssom_write("margin-block", "1px 2px", false, None)
            .expect("CSSOM margin-block should parse through the PDB value-fragment path");
    assert!(
        cssom_style_property_write_uses_pdb("margin-block", "1px 2px"),
        "logical box shorthand CSSOM writes should no longer fall back to the entries adapter"
    );
    assert_eq!(
        cssom_margin_block
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [("margin-block-start", "1px"), ("margin-block-end", "2px")]
    );
    assert!(
        cssom_margin_block
            .entries
            .iter()
            .all(style_entry_is_pdb_safe)
    );

    let cssom_margin_inline_start = parse_style_property_entries_for_cssom_write(
        "margin-inline-start",
        "clamp(1px,2px,3px)",
        false,
        None,
    )
    .expect("CSSOM margin-inline-start should parse through the PDB value-fragment path");
    assert!(cssom_style_property_write_uses_pdb(
        "margin-inline-start",
        "clamp(1px,2px,3px)"
    ));
    assert_eq!(cssom_margin_inline_start.entries.len(), 1);
    assert_eq!(
        cssom_margin_inline_start.entries[0].name,
        "margin-inline-start"
    );
    assert_eq!(cssom_margin_inline_start.entries[0].value, "calc(2px)");
    assert!(style_entry_is_pdb_safe(
        &cssom_margin_inline_start.entries[0]
    ));

    let cssom_padding_left = parse_style_property_entries_for_cssom_write(
        "padding-left",
        "calc(calc(12px))",
        false,
        None,
    )
    .expect("CSSOM padding-left should parse through the PDB value-fragment path");
    assert!(
        cssom_style_property_write_uses_pdb("padding-left", "calc(calc(12px))"),
        "physical padding longhand CSSOM writes should no longer fall back to the entries adapter"
    );
    assert_eq!(cssom_padding_left.entries.len(), 1);
    assert_eq!(cssom_padding_left.entries[0].name, "padding-left");
    assert_eq!(cssom_padding_left.entries[0].value, "calc(12px)");
    assert!(style_entry_is_pdb_safe(&cssom_padding_left.entries[0]));

    let cssom_padding_inline_end = parse_style_property_entries_for_cssom_write(
        "padding-inline-end",
        "calc(calc(12px))",
        false,
        None,
    )
    .expect("CSSOM padding-inline-end should parse through the PDB value-fragment path");
    assert!(cssom_style_property_write_uses_pdb(
        "padding-inline-end",
        "calc(calc(12px))"
    ));
    assert_eq!(cssom_padding_inline_end.entries.len(), 1);
    assert_eq!(
        cssom_padding_inline_end.entries[0].name,
        "padding-inline-end"
    );
    assert_eq!(cssom_padding_inline_end.entries[0].value, "calc(12px)");
    assert!(style_entry_is_pdb_safe(
        &cssom_padding_inline_end.entries[0]
    ));

    for (property, value, expected_value) in [
        ("background-size", "10px 20px", "10px 20px"),
        ("block-size", "clamp(1px,2px,3px)", "calc(2px)"),
        ("letter-spacing", "clamp(1px,2px,3px)", "calc(2px)"),
        ("opacity", "0.5", "0.5"),
        ("opacity", "50%", "0.5"),
        ("opacity", "calc(-50% - 50%)", "calc(-100%)"),
        ("opacity", "clamp(50%,80%,70%)", "clamp(50%, 80%, 70%)"),
        ("opacity", "calc(-0.5 - 0.5)", "calc(-1)"),
        ("rotate", "45deg", "45deg"),
        ("scale", "2", "2"),
        ("tab-size", "4", "4"),
        ("text-indent", "calc(calc(12px))", "calc(12px)"),
        ("z-index", "3", "3"),
    ] {
        let parsed = parse_style_property_entries_for_cssom_write(property, value, false, None)
            .unwrap_or_else(|| {
                panic!("{property}: {value} should parse through the PDB value-fragment path")
            });
        assert!(
            cssom_style_property_write_uses_pdb(property, value),
            "{property}: {value} should no longer fall back to the renderer entries adapter"
        );
        assert_eq!(parsed.entries.len(), 1, "{property}: {value}");
        assert_eq!(parsed.entries[0].name, property, "{property}: {value}");
        assert_eq!(
            parsed.entries[0].value, expected_value,
            "{property}: {value}"
        );
        assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
        assert!(
            parse_style_property_entries_with_pdb(property, value, false).is_some(),
            "{property}: {value} should parse directly through PDB"
        );
    }

    for (property, value) in [
        ("background-size", "1px 2px 3px"),
        ("block-size", "banana"),
        ("letter-spacing", "1px 2px"),
        ("opacity", "banana"),
        ("rotate", "1px"),
        ("scale", "banana"),
        ("tab-size", "-1"),
        ("text-indent", "banana"),
        ("z-index", "1.5"),
    ] {
        assert!(
            parse_style_property_entries_for_cssom_write(property, value, false, None).is_none(),
            "{property}: {value} should be rejected by the PDB value-fragment path"
        );
        assert!(
            parse_style_property_entries_with_pdb(property, value, false).is_none(),
            "{property}: {value} should be rejected by direct PDB parsing"
        );
    }

    let width =
        parse_style_property_entries_with_base("width", "calc(10px + 1vmin + 10%)", false, None)
            .expect("valid calc width should parse");
    assert_eq!(width.entries.len(), 1);
    assert_eq!(width.entries[0].name, "width");
    assert_eq!(width.entries[0].value, "calc(10% + 10px + 1vmin)");

    let margin =
        parse_style_property_entries_with_base("margin-top", "clamp(1px,2px,3px)", false, None)
            .expect("valid clamp margin should parse");
    assert_eq!(margin.entries.len(), 1);
    assert_eq!(margin.entries[0].name, "margin-top");
    assert_eq!(margin.entries[0].value, "calc(2px)");

    let margin_shorthand = parse_style_property_entries_with_base("margin", "1px", false, None)
        .expect("valid margin shorthand should parse");
    assert_eq!(margin_shorthand.entries.len(), 4);
    assert_eq!(margin_shorthand.entries[0].name, "margin-top");
    assert_eq!(margin_shorthand.entries[0].value, "1px");
    assert_eq!(margin_shorthand.entries[1].name, "margin-right");
    assert_eq!(margin_shorthand.entries[1].value, "1px");
    assert_eq!(margin_shorthand.entries[2].name, "margin-bottom");
    assert_eq!(margin_shorthand.entries[2].value, "1px");
    assert_eq!(margin_shorthand.entries[3].name, "margin-left");
    assert_eq!(margin_shorthand.entries[3].value, "1px");

    let padding_shorthand =
        parse_style_property_entries_with_base("padding", "calc(calc(12px))", false, None)
            .expect("valid nested calc padding shorthand should parse");
    assert_eq!(padding_shorthand.entries.len(), 4);
    assert_eq!(padding_shorthand.entries[0].name, "padding-top");
    assert_eq!(padding_shorthand.entries[0].value, "calc(12px)");
    assert_eq!(padding_shorthand.entries[1].name, "padding-right");
    assert_eq!(padding_shorthand.entries[1].value, "calc(12px)");
    assert_eq!(padding_shorthand.entries[2].name, "padding-bottom");
    assert_eq!(padding_shorthand.entries[2].value, "calc(12px)");
    assert_eq!(padding_shorthand.entries[3].name, "padding-left");
    assert_eq!(padding_shorthand.entries[3].value, "calc(12px)");

    let border = parse_style_property_entries_with_base(
        "border",
        "calc(calc(10px)) solid pink",
        false,
        None,
    )
    .expect("valid nested calc border shorthand should parse through PDB");
    assert!(border.entries.iter().all(style_entry_is_pdb_safe));
    for (name, value) in [
        ("border-top-width", "calc(10px)"),
        ("border-right-width", "calc(10px)"),
        ("border-bottom-width", "calc(10px)"),
        ("border-left-width", "calc(10px)"),
        ("border-top-style", "solid"),
        ("border-right-style", "solid"),
        ("border-bottom-style", "solid"),
        ("border-left-style", "solid"),
    ] {
        assert!(
            border
                .entries
                .iter()
                .any(|entry| entry.name == name && entry.value == value),
            "border base fallback should materialize PDB entry {name}: {value}"
        );
    }

    let border_width =
        parse_style_property_entries_with_base("border-top-width", "calc(calc(10px))", false, None)
            .expect("valid nested calc border width longhand should parse through PDB");
    assert!(border_width.entries.iter().all(style_entry_is_pdb_safe));
    assert!(
        border_width
            .entries
            .iter()
            .any(|entry| { entry.name == "border-top-width" && entry.value == "calc(10px)" })
    );

    let border_width_shorthand =
        parse_style_property_entries_with_base("border-width", "calc(calc(12px))", false, None)
            .expect("valid nested calc border-width shorthand should parse through PDB");
    assert!(
        border_width_shorthand
            .entries
            .iter()
            .all(style_entry_is_pdb_safe)
    );
    for longhand in [
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
    ] {
        assert!(
            border_width_shorthand
                .entries
                .iter()
                .any(|entry| entry.name == longhand && entry.value == "calc(12px)"),
            "border-width base fallback should materialize PDB entry {longhand}"
        );
    }

    let border_top = parse_style_property_entries_with_base(
        "border-top",
        "calc(calc(11px)) solid pink",
        false,
        None,
    )
    .expect("valid nested calc border side shorthand should parse through PDB");
    assert!(border_top.entries.iter().all(style_entry_is_pdb_safe));
    for (name, value) in [
        ("border-top-width", "calc(11px)"),
        ("border-top-style", "solid"),
    ] {
        assert!(
            border_top
                .entries
                .iter()
                .any(|entry| entry.name == name && entry.value == value),
            "border-top base fallback should materialize PDB entry {name}: {value}"
        );
    }

    for (property, value) in [
        ("border", "banana"),
        ("border-width", "1px 2px 3px 4px 5px"),
        ("border-top-width", "1px 2px"),
        ("border-image-width", "banana"),
    ] {
        assert!(
            parse_style_property_entries_with_base(property, value, false, None).is_none(),
            "{property}: {value} should be rejected by the PDB-backed base fallback"
        );
    }
}

#[test]
fn content_visibility_cssom_writes_use_stylo_pdb() {
    for (value, expected) in [
        ("hidden", "hidden"),
        ("AUTO", "auto"),
        ("inherit", "inherit"),
    ] {
        assert!(cssom_style_property_write_uses_pdb(
            "content-visibility",
            value
        ));
        let parsed =
            parse_style_property_entries_for_cssom_write("content-visibility", value, true, None)
                .expect("content-visibility should parse through Stylo PDB");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "content-visibility");
        assert_eq!(parsed.entries[0].value, expected);
        assert!(parsed.entries[0].priority);
        assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
    }

    assert!(
        parse_style_property_entries_for_cssom_write("content-visibility", "bogus", false, None,)
            .is_none()
    );
}

#[test]
fn transition_pdb_parser_accepts_dynamic_numeric_longhands() {
    let shorthand = parse_style_property_entries_with_base(
        "transition",
        "display 3s ease-in-out 1s allow-discrete, opacity",
        true,
        None,
    )
    .expect("base parser should route transition shorthand through PDB");
    for longhand in transition_shorthand_longhands() {
        assert!(
            shorthand.affected_names.iter().any(|name| name == longhand),
            "base parser transition should affect {longhand}"
        );
    }
    assert!(
        shorthand.entries.iter().any(|entry| {
            entry.name == "transition-duration" && entry.value == "3s, 0s" && entry.priority
        }),
        "base parser transition should retain PDB longhand projection"
    );
    assert!(
        shorthand.entries.iter().any(|entry| {
            entry.name == "transition-behavior"
                && entry.value == "allow-discrete, normal"
                && entry.priority
        }),
        "base parser transition should include behavior longhand projection"
    );

    let duration = parse_style_property_entries_with_pdb(
        "transition-duration",
        "calc(10s + (sign(2cqw - 10px) * 5s))",
        false,
    )
    .expect("dynamic transition-duration should parse for PDB-backed CSSOM");
    assert_eq!(duration.entries.len(), 1);
    assert_eq!(duration.entries[0].name, "transition-duration");
    assert_eq!(
        duration.entries[0].value,
        "calc(10s + (5s * sign(2cqw - 10px)))"
    );

    let timing = parse_style_property_entries_with_pdb(
        "transition-timing-function",
        "steps(calc(2 * sibling-index()), jump-none)",
        false,
    )
    .expect("dynamic transition-timing-function should parse for PDB-backed CSSOM");
    assert_eq!(timing.entries.len(), 1);
    assert_eq!(timing.entries[0].name, "transition-timing-function");
    assert_eq!(
        timing.entries[0].value,
        "steps(calc(2 * sibling-index()), jump-none)"
    );

    let base_duration = parse_style_property_entries_with_base(
        "transition-duration",
        "calc(10s + (sign(2cqw - 10px) * 5s))",
        false,
        None,
    )
    .expect("base parser should route dynamic transition-duration through PDB");
    assert_eq!(base_duration.entries.len(), 1);
    assert_eq!(base_duration.entries[0].name, "transition-duration");
    assert_eq!(
        base_duration.entries[0].value,
        "calc(10s + (5s * sign(2cqw - 10px)))"
    );
    assert!(!style_entry_is_pdb_supplemental_side_entry(
        &base_duration.entries[0]
    ));

    for (property, value) in [
        ("transition", "1s 2s 3s"),
        ("transition-duration", "-2s"),
        ("transition-property", "none, width"),
    ] {
        assert!(
            parse_style_property_entries_with_base(property, value, false, None).is_none(),
            "{property}: {value} should be rejected by the PDB write boundary"
        );
    }
}

#[test]
fn base_parser_routes_remaining_pdb_families_through_pdb() {
    let animation = parse_style_property_entries_with_base(
        "animation",
        "fade paused both reverse 3 1s 2s linear",
        true,
        None,
    )
    .expect("base parser should route animation shorthand through PDB");
    for longhand in animation_shorthand_longhands() {
        assert!(
            animation.affected_names.iter().any(|name| name == longhand),
            "base parser animation should affect {longhand}"
        );
    }
    for reset_only in [
        "animation-timeline",
        "animation-range-start",
        "animation-range-end",
    ] {
        assert!(
            animation
                .entries
                .iter()
                .any(|entry| entry.name == reset_only && entry.priority),
            "base parser animation should keep reset-only {reset_only} entries"
        );
    }

    let dynamic_duration = parse_style_property_entries_with_base(
        "animation-duration",
        "calc(10s + (sign(2cqw - 10px) * 5s))",
        false,
        None,
    )
    .expect("base parser should route dynamic animation-duration through PDB");
    assert_eq!(dynamic_duration.entries.len(), 1);
    assert_eq!(dynamic_duration.entries[0].name, "animation-duration");
    assert_eq!(
        dynamic_duration.entries[0].value,
        "calc(10s + (5s * sign(2cqw - 10px)))"
    );
    assert!(!style_entry_is_pdb_supplemental_side_entry(
        &dynamic_duration.entries[0]
    ));

    let font = parse_style_property_entries_with_base(
        "font",
        "italic small-caps 700 16px / 2 Ahem",
        true,
        None,
    )
    .expect("base parser should route font shorthand through PDB");
    for longhand in font_shorthand_longhands() {
        assert!(
            font.affected_names.iter().any(|name| name == longhand),
            "base parser font should affect {longhand}"
        );
    }
    assert!(style_entry_is_pdb_safe(&StyleEntry {
        name: "font".to_owned(),
        value: "italic small-caps 700 16px / 2 Ahem".to_owned(),
        priority: true,
    }));

    let stroke =
        parse_style_property_entries_with_base("-webkit-text-stroke", "1px red", true, None)
            .expect("base parser should route -webkit-text-stroke through PDB");
    assert_eq!(
        stroke
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
            .collect::<Vec<_>>(),
        [
            ("-webkit-text-stroke-width", "1px", true),
            ("-webkit-text-stroke-color", "red", true),
        ]
    );

    for (property, value) in [
        ("animation", "1s 2s 3s"),
        ("animation-duration", "-1s"),
        ("font", "16px"),
        ("-webkit-text-stroke", "banana red"),
    ] {
        assert!(
            parse_style_property_entries_with_base(property, value, false, None).is_none(),
            "{property}: {value} should be rejected by the PDB write boundary"
        );
    }
}

#[test]
fn transition_shorthand_query_uses_pdb_longhand_state() {
    let mut block = moli_css_parse::parse_declaration_block(
        "transition: display 3s ease-in-out 1s allow-discrete, opacity !important;",
    );
    let projection = block.set_property_with_projection("transition-duration", "4s, 5s", true);
    assert_ne!(
        projection.set_result,
        moli_css_parse::CssSetResult::ParseError
    );

    assert_eq!(
        pdb_property_value_for_cssom_query_with_side_entries(&block, "transition", &[]).as_deref(),
        Some("display 4s ease-in-out 1s allow-discrete, opacity 5s")
    );
    assert_eq!(
        pdb_property_priority_for_cssom_query_with_side_entries(&block, "transition", &[]),
        Some(true)
    );
}

#[test]
fn animation_pdb_parser_expands_shorthand_and_reset_only_entries() {
    let parsed = parse_style_property_entries_with_pdb(
        "animation",
        "fade paused both reverse 3 1s 2s linear",
        true,
    )
    .expect("animation shorthand should parse through PDB");
    let entries = parsed
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
        .collect::<Vec<_>>();
    assert_eq!(
        entries,
        [
            ("animation-duration", "1s", true),
            ("animation-timing-function", "linear", true),
            ("animation-delay", "2s", true),
            ("animation-iteration-count", "3", true),
            ("animation-direction", "reverse", true),
            ("animation-fill-mode", "both", true),
            ("animation-play-state", "paused", true),
            ("animation-name", "fade", true),
            ("animation-timeline", "auto", true),
            ("animation-range-start", "normal", true),
            ("animation-range-end", "normal", true)
        ]
    );
    assert_eq!(
        parsed.affected_names,
        [
            "animation",
            "animation-duration",
            "animation-timing-function",
            "animation-delay",
            "animation-iteration-count",
            "animation-direction",
            "animation-fill-mode",
            "animation-play-state",
            "animation-name",
            "animation-timeline",
            "animation-range-start",
            "animation-range-end"
        ]
    );

    let range =
        parse_style_property_entries_with_pdb("animation-range", "entry 10% exit 20%", false)
            .expect("animation-range shorthand should parse through PDB");
    assert_eq!(
        range
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [
            ("animation-range-start", "entry 10%"),
            ("animation-range-end", "exit 20%")
        ]
    );

    let timeline = parse_style_property_entries_with_pdb("animation-timeline", "auto", false)
        .expect("reset-only animation longhands should parse through PDB supplementals");
    assert_eq!(
        timeline
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [("animation-timeline", "auto")]
    );
    let range_start =
        parse_style_property_entries_with_pdb("animation-range-start", "normal", false)
            .expect("animation-range-start reset value should parse through PDB supplementals");
    assert_eq!(range_start.entries[0].value, "normal");
}

#[test]
fn animation_pdb_parser_keeps_only_required_supplemental_side_entries() {
    let animation = style_entry("animation", "fade 1s linear");
    let range = style_entry("animation-range", "entry 10% exit 20%");
    let simple_duration = style_entry("animation-duration", "1s");
    let ordinary_timing = style_entry("animation-timing-function", "linear");
    let dynamic_duration =
        style_entry("animation-duration", "calc(10s + (sign(2cqw - 10px) * 5s))");
    let cssom_timing = style_entry("animation-timing-function", "linear(0, 1)");

    assert!(style_entry_is_pdb_safe(&animation));
    assert!(style_entry_is_pdb_safe(&range));
    assert!(style_entry_is_pdb_safe(&simple_duration));
    assert!(style_entry_is_pdb_safe(&ordinary_timing));
    assert!(
        !style_entry_is_pdb_supplemental_side_entry(&simple_duration),
        "ordinary animation-duration should stay only in the PDB block"
    );
    assert!(
        !style_entry_is_pdb_supplemental_side_entry(&ordinary_timing),
        "ordinary animation-timing-function should stay only in the PDB block"
    );
    assert!(
        !style_entry_is_pdb_supplemental_side_entry(&dynamic_duration),
        "dynamic animation-duration should stay in Stylo's PDB block"
    );
    assert!(
        style_entry_is_pdb_supplemental_side_entry(&cssom_timing),
        "animation-timing-function keeps CSSOM-compatible easing text"
    );

    let state = inline_style_declaration_state_from_entries(&[
        style_entry("animation-name", "fade"),
        dynamic_duration.clone(),
        cssom_timing.clone(),
    ]);
    assert_eq!(
        state
            .side_entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [(cssom_timing.name.as_str(), cssom_timing.value.as_str())]
    );
    assert_eq!(
        state.block.property_value("animation-duration").as_deref(),
        Some("calc(10s + (5s * sign(2cqw - 10px)))")
    );
    assert!(
        state
            .block
            .property_value("animation-timing-function")
            .is_none_or(|value| value.is_empty())
    );

    let ordinary_state = inline_style_declaration_state_from_entries(&[ordinary_timing]);
    assert!(
        ordinary_state.side_entries.is_empty(),
        "ordinary animation timing values should not keep supplemental side storage"
    );
    assert_eq!(
        ordinary_state
            .block
            .property_value("animation-timing-function")
            .as_deref(),
        Some("linear")
    );
}

#[test]
fn font_variant_pdb_query_ignores_unrelated_block_entries() {
    let state = inline_style_declaration_state_from_entries(&[style_entry("color", "red")]);

    assert_eq!(
        inline_state_property_value_with_pdb(&state, "font-variant"),
        None
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "font-variant"),
        None
    );
}

#[test]
fn inline_pdb_query_ignores_unrelated_block_entries() {
    let state = inline_style_declaration_state_from_entries(&[style_entry("color", "red")]);

    assert_eq!(
        inline_state_property_value_with_pdb(&state, "display"),
        None
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "display"),
        None
    );
}

#[test]
fn css_color_properties_use_pdb_entries() {
    for property in ["accent-color", "background-color", "caret-color", "color"] {
        let parsed = parse_style_property_entries_for_cssom_write(
            property,
            "rgb(0 128 0 / 50%)",
            false,
            None,
        )
        .unwrap_or_else(|| panic!("{property} should accept a valid rgb color"));
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, property);
        assert_eq!(parsed.entries[0].value, "rgba(0, 128, 0, 0.5)");
        assert!(
            parse_style_property_entries_with_pdb(property, "rgb(0 128 0 / 50%)", false).is_some(),
            "{property} should be owned by PDB for CSSOM color writes"
        );
        assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
        assert!(
            parse_style_property_entries_for_cssom_write(
                property,
                "rgb(clamp(10, none, 20) 0 0)",
                false,
                None,
            )
            .is_none(),
            "{property} should reject invalid math inside rgb()"
        );
    }

    for (property, value) in [("accent-color", "auto"), ("caret-color", "auto")] {
        let parsed = parse_style_property_entries_for_cssom_write(property, value, true, None)
            .unwrap_or_else(|| panic!("{property}: {value} should parse through PDB"));
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, property);
        assert_eq!(parsed.entries[0].value, value);
        assert!(parsed.entries[0].priority);
        assert!(parse_style_property_entries_with_pdb(property, value, true).is_some());
        assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
    }
}

#[test]
fn unresolved_legacy_color_functions_use_canonical_component_units() {
    for (input, expected) in [
        (
            "rgba(calc(50 + (sign(1em - 10px) * 10)) 400% -400% / 50%)",
            "rgb(calc(50 + (10 * sign(1em - 10px))) 255 0 / 0.5)",
        ),
        (
            "hsla(calc(50deg + (sign(1em - 10px) * 10deg)) -100% 300% / 50%)",
            "hsl(calc(50deg + (10deg * sign(1em - 10px))) 0 300 / 0.5)",
        ),
        (
            "hwb(calc(110deg + (sign(1em - 10px) * 10deg)) 30% 50% / 50%)",
            "hwb(calc(110deg + (10deg * sign(1em - 10px))) 30 50 / 0.5)",
        ),
        (
            "hwb(120deg 30% 50% / calc(50% + (sign(1em - 10px) * 10%)))",
            "hwb(120 30 50 / calc(50% + (10% * sign(1em - 10px))))",
        ),
    ] {
        let parsed = parse_style_property_entries_for_cssom_write("color", input, false, None)
            .unwrap_or_else(|| panic!("color should accept {input}"));
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].value, expected);

        let mut block = moli_css_parse::CssDeclarationBlock::default();
        assert_ne!(
            block.set_property("color", input, false),
            moli_css_parse::CssSetResult::ParseError
        );
        assert_eq!(
            pdb_property_value_for_cssom_query_with_side_entries(&block, "color", &[])
                .as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn background_image_serializes_resolution_math_with_stylo_parser() {
    let parsed = parse_style_property_entries_with_base(
        "background-image",
        r#"image-set(url("") calc(1x * NaN))"#,
        false,
        None,
    )
    .expect("valid image-set resolution math should parse");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].name, "background-image");
    assert_eq!(
        parsed.entries[0].value,
        r#"image-set(url("") calc(NaN * 1dppx))"#
    );
}

#[test]
fn pdb_parser_accepts_gap_shorthand_entries() {
    let block = moli_css_parse::parse_declaration_block("gap: 10px 10px;");
    assert_eq!(block.property_value("gap").as_deref(), Some("10px"));

    let parsed = parse_style_property_entries_with_pdb("gap", "10px 10px", false)
        .expect("gap shorthand should parse through PDB");
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries[0].name, "row-gap");
    assert_eq!(parsed.entries[0].value, "10px");
    assert_eq!(parsed.entries[1].name, "column-gap");
    assert_eq!(parsed.entries[1].value, "10px");

    for name in ["row-gap", "column-gap"] {
        let parsed = parse_style_property_entries_for_cssom_write(name, "567px", false, None)
            .unwrap_or_else(|| panic!("{name} should parse through PDB"));
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, name);
        assert_eq!(parsed.entries[0].value, "567px");
        assert!(
            parse_style_property_entries_for_cssom_write(name, "1234", false, None).is_none(),
            "{name} should reject non-zero unitless CSSOM lengths"
        );
    }
}

#[test]
fn inline_css_text_all_adapter_removes_preceding_reset_properties() {
    let state =
        inline_css_text_pdb_storage_state("display: block; all: inherit; padding-left: 1px;")
            .expect("all cssText should build a PDB storage state");

    assert_eq!(
        state
            .entries
            .into_iter()
            .map(|entry| (entry.name, entry.value))
            .collect::<Vec<_>>(),
        [
            ("all".to_owned(), "inherit".to_owned()),
            ("padding-left".to_owned(), "1px".to_owned())
        ]
    );
}

#[test]
fn inline_dynamic_transition_entries_seed_pdb_without_css_text_reparse() {
    let duration = style_entry(
        "transition-duration",
        "calc(10s + (sign(2cqw - 10px) * 5s))",
    );
    let timing = style_entry(
        "transition-timing-function",
        "steps(calc(2 * sibling-index()), jump-none)",
    );
    assert!(
        inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(&[
            duration.clone(),
            timing.clone()
        ])
    );

    let state = inline_style_declaration_state_from_entries(&[duration, timing]);
    assert!(!state.block.is_empty());
    assert!(state.side_entries.is_empty());
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "transition-duration").as_deref(),
        Some("calc(10s + (5s * sign(2cqw - 10px)))")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "transition-timing-function").as_deref(),
        Some("steps(calc(2 * sibling-index()), jump-none)")
    );
}

#[test]
fn inline_pdb_mutation_can_replace_fully_covered_side_entries() {
    let shorthand_state = CssInlineStyleDeclarationState {
        entries: vec![
            style_entry("--token", "value"),
            style_entry("padding-left", "var(--pad)"),
        ],
        side_entries: vec![
            style_entry("--token", "value"),
            style_entry("padding-left", "var(--pad)"),
        ],
        ..Default::default()
    };
    let padding_affected = style_property_affected_names_with_pdb("padding")
        .expect("padding affected names should come from PDB");
    assert!(
        !inline_state_has_unpreservable_side_entries_for_property(
            &shorthand_state,
            "padding",
            &padding_affected,
        ),
        "padding shorthand should be able to replace a legacy padding-left side entry"
    );
    assert!(
        inline_state_has_replaceable_side_entries_for_property(
            &shorthand_state,
            "padding",
            &padding_affected,
        ),
        "empty padding writes should still notice the replaceable side entry"
    );

    let longhand_state = CssInlineStyleDeclarationState {
        entries: vec![style_entry("padding", "var(--pad)")],
        side_entries: vec![style_entry("padding", "var(--pad)")],
        ..Default::default()
    };
    let padding_left_affected = style_property_affected_names_with_pdb("padding-left")
        .expect("padding-left affected names should come from PDB");
    assert!(
        parse_style_property_entries_with_pdb("padding-left", "1px", false).is_some(),
        "padding-left should be writable through PDB"
    );
    assert!(
        !inline_state_has_unpreservable_side_entries_for_property(
            &longhand_state,
            "padding-left",
            &padding_left_affected,
        ),
        "padding-left can preserve a legacy padding shorthand side entry while writing PDB"
    );
    assert!(
        !inline_state_has_replaceable_side_entries_for_property(
            &longhand_state,
            "padding-left",
            &padding_left_affected,
        ),
        "padding-left must not replace a legacy padding shorthand side entry"
    );

    let border_longhand_state = CssInlineStyleDeclarationState {
        entries: vec![style_entry("border-width", "var(--w)")],
        side_entries: vec![style_entry("border-width", "var(--w)")],
        ..Default::default()
    };
    let border_left_width_affected = style_property_affected_names_with_pdb("border-left-width")
        .expect("border-left-width affected names should come from PDB");
    assert!(
        parse_style_property_entries_with_pdb("border-left-width", "1px", false).is_some(),
        "border-left-width should be writable through PDB"
    );
    assert!(
        !inline_state_has_unpreservable_side_entries_for_property(
            &border_longhand_state,
            "border-left-width",
            &border_left_width_affected,
        ),
        "border-left-width can preserve a legacy border-width shorthand side entry"
    );
    assert!(
        !inline_state_has_replaceable_side_entries_for_property(
            &border_longhand_state,
            "border-left-width",
            &border_left_width_affected,
        ),
        "border-left-width must not replace a legacy border-width shorthand side entry"
    );

    let border_side_state = CssInlineStyleDeclarationState {
        entries: vec![style_entry("border-top", "var(--top)")],
        side_entries: vec![style_entry("border-top", "var(--top)")],
        ..Default::default()
    };
    let border_top_width_affected = style_property_affected_names_with_pdb("border-top-width")
        .expect("border-top-width affected names should come from PDB");
    assert!(
        parse_style_property_entries_with_pdb("border-top-width", "1px", false).is_some(),
        "border-top-width should be writable through PDB"
    );
    assert!(
        !inline_state_has_unpreservable_side_entries_for_property(
            &border_side_state,
            "border-top-width",
            &border_top_width_affected,
        ),
        "border-top-width can preserve a legacy border-top shorthand side entry"
    );
    assert!(
        !inline_state_has_replaceable_side_entries_for_property(
            &border_side_state,
            "border-top-width",
            &border_top_width_affected,
        ),
        "border-top-width must not replace a legacy border-top shorthand side entry"
    );

    let font_state = CssInlineStyleDeclarationState {
        entries: vec![style_entry("font", "var(--font)")],
        side_entries: vec![style_entry("font", "var(--font)")],
        ..Default::default()
    };
    let font_size_affected = style_property_affected_names_with_pdb("font-size")
        .expect("font-size affected names should come from PDB");
    assert!(
        inline_state_has_unpreservable_side_entries_for_property(
            &font_state,
            "font-size",
            &font_size_affected,
        ),
        "font shorthand partial coverage is still outside the proven adapter boundary"
    );
}

#[test]
fn pdb_affected_names_cover_shorthand_families() {
    fn assert_affected_names_include(property: &str, expected: &[&str]) {
        let affected_names = style_property_affected_names_with_pdb(property)
            .unwrap_or_else(|| panic!("{property} should have PDB affected names"));
        for expected_name in expected {
            assert!(
                affected_names
                    .iter()
                    .any(|affected_name| affected_name == expected_name),
                "{property} should affect {expected_name}; got {affected_names:?}"
            );
        }
    }

    let font_affected = style_property_affected_names_with_pdb("font")
        .expect("font should have PDB affected names");
    for expected_name in font_shorthand_longhands()
        .iter()
        .copied()
        .chain(["font", "font-variant"])
    {
        assert!(
            font_affected
                .iter()
                .any(|affected| affected == expected_name),
            "font should affect {expected_name}; got {font_affected:?}"
        );
    }

    let font_variant_affected = style_property_affected_names_with_pdb("font-variant")
        .expect("font-variant should have PDB affected names");
    for expected_name in font_variant_longhands()
        .iter()
        .copied()
        .chain(["font-variant"])
    {
        assert!(
            font_variant_affected
                .iter()
                .any(|affected| affected == expected_name),
            "font-variant should affect {expected_name}; got {font_variant_affected:?}"
        );
    }

    assert_affected_names_include(
        "animation",
        &[
            "animation",
            "animation-name",
            "animation-duration",
            "animation-timeline",
            "animation-range-start",
            "animation-range-end",
        ],
    );
    assert_affected_names_include(
        "border",
        &[
            "border",
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
            "border-image",
        ],
    );
    assert_affected_names_include(
        "overscroll-behavior",
        &[
            "overscroll-behavior",
            "overscroll-behavior-x",
            "overscroll-behavior-y",
        ],
    );
    assert_eq!(
        style_property_affected_names_with_pdb("overscroll-behavior-block"),
        Some(vec!["overscroll-behavior-block".to_owned()])
    );
    assert_eq!(
        style_property_affected_names_with_pdb("overscroll-behavior-inline"),
        Some(vec!["overscroll-behavior-inline".to_owned()])
    );
}

#[test]
fn inline_state_pdb_queries_use_stored_block_when_it_wins_order() {
    let no_side_state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block(
            "display: block; padding-left: 1px !important;",
        ),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&no_side_state, "display").as_deref(),
        Some("block")
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&no_side_state, "padding-left"),
        Some(true)
    );

    let partial_side_state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block("padding-left: 1px;"),
        entries: vec![
            style_entry("padding", "var(--pad)"),
            style_entry("padding-left", "1px"),
        ],
        side_entries: vec![style_entry("padding", "var(--pad)")],
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&partial_side_state, "padding-left").as_deref(),
        Some("1px")
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&partial_side_state, "padding-left"),
        Some(false)
    );

    let important_side_state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block("padding-left: 1px;"),
        entries: vec![
            StyleEntry {
                priority: true,
                ..style_entry("padding", "var(--pad)")
            },
            style_entry("padding-left", "1px"),
        ],
        side_entries: vec![StyleEntry {
            priority: true,
            ..style_entry("padding", "var(--pad)")
        }],
    };
    assert!(
        inline_state_property_value_with_pdb(&important_side_state, "padding-left").is_none(),
        "important side shorthand must keep the query on the CSSOM adapter path"
    );
    assert!(
        inline_state_property_priority_with_pdb(&important_side_state, "padding-left").is_none(),
        "important side shorthand must keep priority on the CSSOM adapter path"
    );

    let overflow_state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block(
            "overflow-x: hidden !important; overflow-y: scroll !important;",
        ),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&overflow_state, "overflow").as_deref(),
        Some("hidden scroll")
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&overflow_state, "overflow"),
        Some(true)
    );

    let animation_state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block("animation: spin 1s;"),
        ..Default::default()
    };
    assert!(
        inline_state_property_value_with_pdb(&animation_state, "animation").is_none(),
        "non-whitelisted shorthand queries must keep their CSSOM adapter semantics"
    );
}

#[test]
fn inline_pdb_mutation_empty_return_uses_mutated_block_entries() {
    let mut state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block(
            "display: block; color: rgb(0 128 0 / 50%);",
        ),
        entries: vec![
            style_entry("display", "block"),
            style_entry("--token", "value"),
            style_entry("color", "red"),
        ],
        side_entries: vec![style_entry("--token", "value")],
    };
    let affected_names = style_property_affected_names_with_pdb("color")
        .expect("color affected names should come from PDB");
    let rebuilt_entries =
        inline_state_block_entries_for_property_mutation(&state, "color", &affected_names);

    assert_eq!(rebuilt_entries.len(), 1);
    assert_eq!(rebuilt_entries[0].name, "color");
    assert_eq!(rebuilt_entries[0].value, "rgba(0, 128, 0, 0.5)");

    refresh_inline_state_entries_after_pdb_mutation(
        &mut state,
        "color",
        &affected_names,
        rebuilt_entries,
        Vec::<StyleEntry>::new(),
    );

    assert_eq!(
        state
            .entries()
            .into_iter()
            .map(|entry| (entry.name, entry.value))
            .collect::<Vec<_>>(),
        [
            ("display".to_owned(), "block".to_owned()),
            ("--token".to_owned(), "value".to_owned()),
            ("color".to_owned(), "rgba(0, 128, 0, 0.5)".to_owned()),
        ]
    );
    assert_eq!(
        state.css_text(),
        "display: block; --token: value; color: rgba(0, 128, 0, 0.5);"
    );
}

#[test]
fn inline_pdb_transition_longhand_mutation_keeps_shorthand_query_with_side_entries() {
    let mut state = inline_style_declaration_state_from_entries(&[
        important_style_entry("transition-property", "display, opacity"),
        important_style_entry("transition-duration", "3s, 0s"),
        important_style_entry("transition-timing-function", "ease-in-out, ease"),
        important_style_entry("transition-delay", "1s, 0s"),
        important_style_entry("transition-behavior", "allow-discrete, normal"),
        style_entry("--token", "value"),
        style_entry("-webkit-transform-origin", "20px 30px"),
    ]);
    assert!(state.block.property_is_declared("transition-property"));
    assert!(state.block.property_is_declared("transition-duration"));
    assert_eq!(state.side_entries.len(), 1);
    assert_eq!(state.side_entries[0].name, "-webkit-transform-origin");
    assert_eq!(state.side_entries[0].value, "20px 30px");

    let affected_names = style_property_affected_names_with_pdb("transition-duration")
        .expect("transition-duration affected names should come from PDB");
    let parsed = parse_style_property_entries_with_pdb("transition-duration", "4s, 5s", true)
        .expect("transition-duration should parse through PDB");
    let entries = set_pdb_block_property_collecting_entries(
        &mut state.block,
        "transition-duration",
        "4s, 5s",
        true,
        &parsed,
        false,
    )
    .expect("transition-duration should update the PDB block");

    refresh_inline_state_entries_after_pdb_mutation(
        &mut state,
        "transition-duration",
        &affected_names,
        entries,
        Vec::<StyleEntry>::new(),
    );

    assert!(state.block.property_is_declared("transition-property"));
    assert!(state.block.property_is_declared("transition-duration"));
    assert_eq!(
        state.block.property_value("transition").as_deref(),
        Some("display 4s ease-in-out 1s allow-discrete, opacity 5s")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "transition").as_deref(),
        Some("display 4s ease-in-out 1s allow-discrete, opacity 5s")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "--token").as_deref(),
        Some("value")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "-webkit-transform-origin").as_deref(),
        None
    );
    assert_eq!(state.side_entries.len(), 1);
    assert_eq!(state.side_entries[0].name, "-webkit-transform-origin");
    assert_eq!(state.side_entries[0].value, "20px 30px");
}

#[test]
fn border_shorthand_is_pdb_write_and_query_safe() {
    let parsed = parse_style_property_entries_with_pdb("border", "1px solid red", true)
        .expect("border shorthand should parse through PDB");
    for longhand in [
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
        "border-top-style",
        "border-right-style",
        "border-bottom-style",
        "border-left-style",
        "border-top-color",
        "border-right-color",
        "border-bottom-color",
        "border-left-color",
        "border-image-source",
        "border-image-slice",
        "border-image-width",
        "border-image-outset",
        "border-image-repeat",
    ] {
        assert!(
            parsed.entries.iter().any(|entry| entry.name == longhand),
            "border should materialize {longhand} through PDB"
        );
        assert!(
            parsed.affected_names.iter().any(|name| name == longhand),
            "border should affect {longhand}"
        );
    }
    assert!(
        parsed.affected_names.iter().any(|name| name == "border"),
        "border should affect its shorthand query"
    );
    assert!(
        parsed
            .affected_names
            .iter()
            .any(|name| name == "border-image"),
        "border should replace legacy border-image side entries"
    );
    assert!(
        style_entry_is_pdb_safe(&StyleEntry {
            name: "border".to_owned(),
            value: "1px solid red".to_owned(),
            priority: true,
        }),
        "border shorthand is now owned by Stylo/PDB"
    );

    let state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block("border: 1px solid red !important;"),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "border").as_deref(),
        Some("1px solid red"),
        "border shorthand query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "border"),
        Some(true),
        "border shorthand priority should be served from PDB"
    );
}

#[test]
fn border_image_shorthand_is_pdb_write_and_query_safe() {
    let parsed = parse_style_property_entries_with_pdb(
        "border-image",
        r#"url("img.png") 30 / 2 / 1 round"#,
        true,
    )
    .expect("border-image should parse through PDB");
    assert_eq!(
        parsed.affected_names,
        [
            "border-image",
            "border-image-outset",
            "border-image-repeat",
            "border-image-slice",
            "border-image-source",
            "border-image-width",
        ]
    );
    assert_eq!(
        parsed
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
            .collect::<Vec<_>>(),
        [
            ("border-image-outset", "1", true),
            ("border-image-repeat", "round", true),
            ("border-image-slice", "30", true),
            ("border-image-source", r#"url("img.png")"#, true),
            ("border-image-width", "2", true),
        ]
    );
    assert!(
        style_entry_is_pdb_safe(&StyleEntry {
            name: "border-image".to_owned(),
            value: r#"url("img.png") 30 / 2 / 1 round"#.to_owned(),
            priority: true,
        }),
        "border-image should no longer be a legacy side-table shorthand"
    );
    let state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block(
            r#"border-image: url("img.png") 30 / 2 / 1 round !important;"#,
        ),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "border-image").as_deref(),
        Some(r#"url("img.png") 30 / 2 / 1 round"#),
        "border-image shorthand query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "border-image"),
        Some(true),
        "border-image shorthand priority should be served from PDB"
    );
    for (property, expected) in [
        ("border-image-outset", "1"),
        ("border-image-repeat", "round"),
        ("border-image-slice", "30"),
        ("border-image-source", r#"url("img.png")"#),
        ("border-image-width", "2"),
    ] {
        assert_eq!(
            inline_state_property_value_with_pdb(&state, property).as_deref(),
            Some(expected),
            "border-image longhand projection should be queryable through PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, property),
            Some(true),
            "border-image longhand projection should retain shorthand priority"
        );
    }
}

#[test]
fn webkit_text_stroke_shorthand_is_pdb_write_and_query_safe() {
    let parsed = parse_style_property_entries_with_pdb("-webkit-text-stroke", "1px red", true)
        .expect("-webkit-text-stroke shorthand should parse through PDB");
    assert_eq!(
        parsed
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
            .collect::<Vec<_>>(),
        [
            ("-webkit-text-stroke-width", "1px", true),
            ("-webkit-text-stroke-color", "red", true),
        ]
    );
    assert_eq!(
        parsed.affected_names,
        [
            "-webkit-text-stroke",
            "-webkit-text-stroke-width",
            "-webkit-text-stroke-color",
        ]
    );
    assert!(style_entry_is_pdb_safe(&StyleEntry {
        name: "-webkit-text-stroke".to_owned(),
        value: "1px red".to_owned(),
        priority: true,
    }));

    let state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block("-webkit-text-stroke: 1px red !important;"),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "-webkit-text-stroke").as_deref(),
        Some("1px red")
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "-webkit-text-stroke"),
        Some(true)
    );
}

#[test]
fn border_component_shorthands_are_pdb_write_and_query_safe() {
    for (property, value, expected) in [
        (
            "border-color",
            "red blue",
            (
                "red blue",
                [
                    "border-top-color",
                    "border-right-color",
                    "border-bottom-color",
                    "border-left-color",
                ],
            ),
        ),
        (
            "border-style",
            "solid dotted",
            (
                "solid dotted",
                [
                    "border-top-style",
                    "border-right-style",
                    "border-bottom-style",
                    "border-left-style",
                ],
            ),
        ),
        (
            "border-width",
            "1px 2px",
            (
                "1px 2px",
                [
                    "border-top-width",
                    "border-right-width",
                    "border-bottom-width",
                    "border-left-width",
                ],
            ),
        ),
    ] {
        assert!(
            parse_style_property_entries_with_pdb(property, value, true).is_some(),
            "{property} should parse through the Stylo declaration block path"
        );
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: property.to_owned(),
                value: value.to_owned(),
                priority: true,
            }),
            "{property} should no longer be a legacy side-table entry"
        );
        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(&format!(
                "{property}: {value} !important;"
            )),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, property).as_deref(),
            Some(expected.0),
            "{property} shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, property),
            Some(true),
            "{property} shorthand priority should be served from PDB"
        );
        let affected = style_property_affected_names_with_pdb(property)
            .expect("border component shorthand affected names should come from PDB");
        let expected_names = expected
            .1
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut expected_affected = vec![property.to_owned()];
        expected_affected.extend(expected_names.iter().cloned());
        assert_eq!(affected, expected_affected);
        for longhand in expected_names {
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: longhand.to_owned(),
                    value: "initial".to_owned(),
                    priority: false,
                }),
                "{longhand} should remain PDB-safe after shorthand expansion"
            );
        }
    }
}

#[test]
fn border_side_shorthands_are_pdb_write_and_query_safe() {
    for (property, value, expected_longhands) in [
        (
            "border-top",
            "1px solid red",
            ["border-top-width", "border-top-style", "border-top-color"],
        ),
        (
            "border-right",
            "2px dotted blue",
            [
                "border-right-width",
                "border-right-style",
                "border-right-color",
            ],
        ),
        (
            "border-bottom",
            "3px dashed green",
            [
                "border-bottom-width",
                "border-bottom-style",
                "border-bottom-color",
            ],
        ),
        (
            "border-left",
            "4px double black",
            [
                "border-left-width",
                "border-left-style",
                "border-left-color",
            ],
        ),
    ] {
        let parsed = parse_style_property_entries_with_pdb(property, value, true)
            .expect("border side shorthand should parse through PDB");
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: property.to_owned(),
                value: value.to_owned(),
                priority: true,
            }),
            "{property} serialized entries should now seed PDB storage"
        );
        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(&format!(
                "{property}: {value} !important;"
            )),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, property).as_deref(),
            Some(value),
            "{property} shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, property),
            Some(true),
            "{property} shorthand priority should be served from PDB"
        );
        let mut expected_affected = vec![property.to_owned()];
        expected_affected.extend(expected_longhands.iter().map(|name| (*name).to_owned()));
        assert_eq!(parsed.affected_names, expected_affected);
        for longhand in expected_longhands {
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: longhand.to_owned(),
                    value: "initial".to_owned(),
                    priority: false,
                }),
                "{longhand} should remain PDB-safe after shorthand expansion"
            );
        }
    }
}

#[test]
fn border_radius_shorthand_is_pdb_write_and_query_safe() {
    let parsed = parse_style_property_entries_with_pdb("border-radius", "1px 2px", true)
        .expect("border-radius should parse through the Stylo declaration block path");
    for longhand in [
        "border-top-left-radius",
        "border-top-right-radius",
        "border-bottom-right-radius",
        "border-bottom-left-radius",
    ] {
        assert!(
            parsed.affected_names.iter().any(|name| name == longhand),
            "border-radius should affect {longhand}"
        );
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: longhand.to_owned(),
                value: "initial".to_owned(),
                priority: false,
            }),
            "{longhand} should be writable through PDB"
        );
    }
    assert!(
        style_entry_is_pdb_safe(&StyleEntry {
            name: "border-radius".to_owned(),
            value: "1px 2px".to_owned(),
            priority: true,
        }),
        "border-radius should no longer be a legacy side-table shorthand"
    );

    let state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block("border-radius: 1px 2px !important;"),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "border-radius").as_deref(),
        Some("1px 2px"),
        "border-radius shorthand query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "border-top-left-radius").as_deref(),
        Some("1px"),
        "border-radius longhand query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "border-radius"),
        Some(true),
        "border-radius shorthand priority should be served from PDB"
    );
}

#[test]
fn text_decoration_family_is_pdb_write_and_query_safe() {
    let parsed = parse_style_property_entries_with_pdb(
        "text-decoration",
        "overline from-font dotted green",
        true,
    )
    .expect("text-decoration should parse through the Stylo declaration block path");
    let expected_longhands = [
        "text-decoration-color",
        "text-decoration-line",
        "text-decoration-style",
        "text-decoration-thickness",
    ];
    let mut expected_affected = vec!["text-decoration".to_owned()];
    expected_affected.extend(expected_longhands.iter().map(|name| (*name).to_owned()));
    assert_eq!(parsed.affected_names, expected_affected);
    let removal_affected = style_property_mutation_affected_names_with_pdb("text-decoration")
        .expect("text-decoration removal should use PDB affected names");
    for name in [
        "text-decoration-color",
        "text-decoration-line",
        "text-decoration-style",
        "text-decoration-thickness",
        "text-decoration-fill",
        "text-decoration-inset",
        "text-decoration-skip-ink",
        "text-decoration-skip-spaces",
        "text-decoration-stroke",
    ] {
        assert!(
            removal_affected.iter().any(|affected| affected == name),
            "text-decoration removal should affect {name}"
        );
    }
    let line_mutation_affected =
        style_property_mutation_affected_names_with_pdb("text-decoration-line")
            .expect("text-decoration-line mutation should use PDB affected names");
    assert!(
        line_mutation_affected
            .iter()
            .any(|affected| affected == "text-decoration-skip-ink"),
        "text-decoration-line mutation should clear text-decoration-skip-ink"
    );
    for (longhand, value) in [
        ("text-decoration-line", "overline"),
        ("text-decoration-thickness", "from-font"),
        ("text-decoration-style", "dotted"),
        ("text-decoration-color", "green"),
        ("text-decoration-fill", "match-text"),
        ("text-decoration-inset", "0px"),
        ("text-decoration-skip-ink", "all"),
        ("text-decoration-skip-spaces", "start end"),
        ("text-decoration-stroke", "context-fill"),
    ] {
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: longhand.to_owned(),
                value: value.to_owned(),
                priority: false,
            }),
            "{longhand} should be writable through PDB"
        );
    }
    assert!(
        style_entry_is_pdb_safe(&StyleEntry {
            name: "text-decoration".to_owned(),
            value: "overline from-font dotted green".to_owned(),
            priority: true,
        }),
        "text-decoration should no longer be a legacy side-table shorthand"
    );
    for compat_keyword in ["spelling-error", "grammar-error"] {
        let compat_entry = StyleEntry {
            name: "text-decoration-line".to_owned(),
            value: compat_keyword.to_owned(),
            priority: false,
        };
        assert!(
            style_entry_is_pdb_safe(&compat_entry),
            "{compat_keyword} should be accepted by the PDB write boundary"
        );
        assert!(
            style_entry_is_pdb_supplemental_side_entry(&compat_entry),
            "{compat_keyword} stays explicit supplemental until Stylo round-trips it natively"
        );
        let parsed = parse_style_property_entries_for_cssom_write(
            "text-decoration-line",
            compat_keyword,
            false,
            None,
        )
        .expect("compat text-decoration-line keyword should remain accepted");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "text-decoration-line");
        assert_eq!(parsed.entries[0].value, compat_keyword);
        assert!(style_entry_is_pdb_supplemental_side_entry(
            &parsed.entries[0]
        ));

        let base_parsed = parse_style_property_entries_with_base(
            "text-decoration-line",
            compat_keyword,
            false,
            None,
        )
        .expect("base parser should route compat text-decoration-line through PDB supplementals");
        assert_eq!(base_parsed.entries.len(), 1);
        assert_eq!(base_parsed.entries[0].name, "text-decoration-line");
        assert_eq!(base_parsed.entries[0].value, compat_keyword);
        assert!(style_entry_is_pdb_supplemental_side_entry(
            &base_parsed.entries[0]
        ));
    }
    let base_normal = parse_style_property_entries_with_base(
        "text-decoration-line",
        "underline overline",
        false,
        None,
    )
    .expect("base parser should route ordinary text-decoration-line through PDB");
    assert_eq!(base_normal.entries.len(), 1);
    assert_eq!(base_normal.entries[0].name, "text-decoration-line");
    assert_eq!(base_normal.entries[0].value, "underline overline");
    assert!(!style_entry_is_pdb_supplemental_side_entry(
        &base_normal.entries[0]
    ));
    assert!(
        parse_style_property_entries_with_base(
            "text-decoration-line",
            "underline underline",
            false,
            None
        )
        .is_none(),
        "base parser should not fall back to renderer text-decoration-line normalization"
    );

    let state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block(
            "text-decoration: overline from-font dotted green !important; text-decoration-skip-ink: all;",
        ),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "text-decoration").as_deref(),
        Some("overline from-font dotted green"),
        "text-decoration shorthand query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "text-decoration-line").as_deref(),
        Some("overline"),
        "text-decoration-line query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "text-decoration-skip-ink").as_deref(),
        Some("all"),
        "text-decoration-skip-ink query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "text-decoration"),
        Some(true),
        "text-decoration shorthand priority should be served from PDB"
    );
}

#[test]
fn text_emphasis_family_is_pdb_write_and_query_safe() {
    let parsed = parse_style_property_entries_with_pdb("text-emphasis", "dot red", true)
        .expect("text-emphasis should parse through the Stylo declaration block path");
    let expected_longhands = ["text-emphasis-style", "text-emphasis-color"];
    let mut expected_affected = vec!["text-emphasis".to_owned()];
    expected_affected.extend(expected_longhands.iter().map(|name| (*name).to_owned()));
    assert_eq!(parsed.affected_names, expected_affected);
    for (longhand, value) in [
        ("text-emphasis-style", "dot"),
        ("text-emphasis-color", "red"),
        ("text-emphasis-position", "over left"),
    ] {
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: longhand.to_owned(),
                value: value.to_owned(),
                priority: false,
            }),
            "{longhand} should be writable through PDB"
        );
    }
    assert!(
        style_entry_is_pdb_safe(&StyleEntry {
            name: "text-emphasis".to_owned(),
            value: "dot red".to_owned(),
            priority: true,
        }),
        "text-emphasis should no longer be a legacy side-table shorthand"
    );
    let base_shorthand =
        parse_style_property_entries_with_base("text-emphasis", "dot red", true, None)
            .expect("base parser should route text-emphasis shorthand through PDB");
    assert_eq!(base_shorthand.affected_names, expected_affected);
    assert_eq!(base_shorthand.entries.len(), 2);
    assert!(
        base_shorthand
            .entries
            .iter()
            .any(|entry| entry.name == "text-emphasis-style" && entry.value == "dot")
    );
    assert!(
        base_shorthand
            .entries
            .iter()
            .any(|entry| entry.name == "text-emphasis-color" && entry.value == "red")
    );
    let base_position =
        parse_style_property_entries_with_base("text-emphasis-position", "over left", false, None)
            .expect("base parser should route text-emphasis-position through PDB");
    assert_eq!(base_position.entries.len(), 1);
    assert_eq!(base_position.entries[0].name, "text-emphasis-position");
    assert_eq!(base_position.entries[0].value, "over left");
    for (property, value) in [
        ("text-emphasis", "filled open"),
        ("text-emphasis-style", "filled open"),
        ("text-emphasis-position", "left right"),
    ] {
        assert!(
            parse_style_property_entries_with_base(property, value, false, None).is_none(),
            "{property}: {value} should be rejected by the PDB write boundary"
        );
    }

    let state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block(
            "text-emphasis: dot red !important; text-emphasis-position: over left;",
        ),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "text-emphasis").as_deref(),
        Some("dot red"),
        "text-emphasis shorthand query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "text-emphasis-style").as_deref(),
        Some("dot"),
        "text-emphasis-style query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "text-emphasis-position").as_deref(),
        Some("over left"),
        "text-emphasis-position query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "text-emphasis"),
        Some(true),
        "text-emphasis shorthand priority should be served from PDB"
    );
}

#[test]
fn font_variant_family_is_pdb_write_and_query_safe() {
    let alternates =
        parse_style_property_entries_with_pdb("font-variant-alternates", "historical-forms", true)
            .expect("font-variant-alternates should parse as a native PDB longhand");
    assert_eq!(alternates.entries.len(), 1);
    assert_eq!(alternates.entries[0].name, "font-variant-alternates");
    assert_eq!(alternates.entries[0].value, "historical-forms");
    assert!(!style_entry_is_pdb_supplemental_side_entry(
        &alternates.entries[0]
    ));

    let parsed = parse_style_property_entries_with_pdb("font-variant", "small-caps", true)
        .expect("font-variant should parse through the Stylo declaration block path");
    assert_eq!(
        parsed.affected_names.first().map(String::as_str),
        Some("font-variant")
    );
    for longhand in [
        "font-variant-ligatures",
        "font-variant-caps",
        "font-variant-alternates",
        "font-variant-numeric",
        "font-variant-east-asian",
        "font-variant-position",
        "font-variant-emoji",
    ] {
        assert!(
            parsed.affected_names.iter().any(|name| name == longhand),
            "font-variant should affect {longhand}"
        );
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: longhand.to_owned(),
                value: "normal".to_owned(),
                priority: false,
            }),
            "{longhand} should be writable through PDB"
        );
    }
    assert!(
        style_entry_is_pdb_safe(&StyleEntry {
            name: "font-variant".to_owned(),
            value: "small-caps".to_owned(),
            priority: true,
        }),
        "font-variant should no longer be a legacy side-table shorthand"
    );

    let base_shorthand =
        parse_style_property_entries_with_base("font-variant", "small-caps", true, None)
            .expect("base parser should route font-variant through PDB");
    for longhand in font_variant_longhands() {
        assert!(
            base_shorthand
                .affected_names
                .iter()
                .any(|name| name == longhand),
            "base parser font-variant should affect {longhand}"
        );
    }
    assert!(
        base_shorthand
            .entries
            .iter()
            .any(|entry| entry.name == "font-variant-caps" && entry.value == "small-caps"),
        "base parser font-variant should retain PDB longhand projection"
    );
    assert!(
        base_shorthand.entries.iter().any(|entry| {
            entry.name == "font-variant-alternates"
                && entry.value == "normal"
                && !style_entry_is_pdb_supplemental_side_entry(entry)
        }),
        "base parser font-variant should include native longhand state"
    );

    let base_alternates = parse_style_property_entries_with_base(
        "font-variant-alternates",
        "historical-forms",
        true,
        None,
    )
    .expect("base parser should route font-variant-alternates through native PDB storage");
    assert_eq!(base_alternates.entries.len(), 1);
    assert_eq!(base_alternates.entries[0].name, "font-variant-alternates");
    assert_eq!(base_alternates.entries[0].value, "historical-forms");
    assert!(!style_entry_is_pdb_supplemental_side_entry(
        &base_alternates.entries[0]
    ));

    for (property, value) in [
        ("font-variant", "small-caps small-caps"),
        ("font-variant-caps", "small-caps petite-caps"),
        ("font-variant-position", "sub super"),
    ] {
        assert!(
            parse_style_property_entries_with_base(property, value, false, None).is_none(),
            "{property}: {value} should be rejected by the PDB write boundary"
        );
    }

    let state = inline_style_declaration_state_from_entries(&[
        StyleEntry {
            priority: true,
            ..style_entry("font-variant", "normal")
        },
        StyleEntry {
            priority: true,
            ..style_entry("font-variant-caps", "small-caps")
        },
        StyleEntry {
            priority: true,
            ..style_entry("font-variant-alternates", "historical-forms")
        },
    ]);
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "font-variant").as_deref(),
        Some("small-caps historical-forms"),
        "font-variant shorthand query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "font-variant"),
        Some(true),
        "font-variant shorthand priority should be served from PDB"
    );

    let none_state = inline_style_declaration_state_from_entries(&[
        StyleEntry {
            priority: true,
            ..style_entry("font-variant", "normal")
        },
        StyleEntry {
            priority: true,
            ..style_entry("font-variant-ligatures", "none")
        },
    ]);
    assert_eq!(
        inline_state_property_value_with_pdb(&none_state, "font-variant").as_deref(),
        Some("none"),
        "font-variant-ligatures:none should serialize as the standalone shorthand none"
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&none_state, "font-variant-ligatures").as_deref(),
        Some("none"),
        "font-variant-ligatures query should be served from PDB"
    );
}

#[test]
fn font_shorthand_is_pdb_write_and_query_safe() {
    let parsed =
        parse_style_property_entries_with_pdb("font", "italic small-caps 700 16px / 2 Ahem", true)
            .expect("font shorthand should parse through the Stylo declaration block path");
    assert_eq!(
        parsed.affected_names.first().map(String::as_str),
        Some("font")
    );
    for longhand in [
        "font-style",
        "font-variant-ligatures",
        "font-variant-caps",
        "font-variant-alternates",
        "font-variant-numeric",
        "font-variant-east-asian",
        "font-variant-position",
        "font-variant-emoji",
        "font-weight",
        "font-stretch",
        "font-size",
        "line-height",
        "font-family",
        "font-kerning",
    ] {
        assert!(
            parsed.affected_names.iter().any(|name| name == longhand),
            "font should affect {longhand}"
        );
    }
    assert!(
        style_entry_is_pdb_safe(&StyleEntry {
            name: "font".to_owned(),
            value: "italic small-caps 700 16px / 2 Ahem".to_owned(),
            priority: true,
        }),
        "font shorthand should no longer be a legacy side-table shorthand"
    );

    let state = inline_style_declaration_state_from_entries(&[StyleEntry {
        priority: true,
        ..style_entry("font", "italic small-caps 700 16px / 2 Ahem")
    }]);
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "font").as_deref(),
        Some("italic small-caps 700 16px / 2 Ahem"),
        "font shorthand query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "font"),
        Some(true),
        "font shorthand priority should be served from PDB"
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "font-size").as_deref(),
        Some("16px"),
        "font-size longhand query should stay on the PDB path"
    );
}

#[test]
fn outline_shorthand_is_pdb_write_and_query_safe() {
    let parsed = parse_style_property_entries_with_pdb("outline", "1px solid red", true)
        .expect("outline should parse through the Stylo declaration block path");
    for longhand in ["outline-color", "outline-style", "outline-width"] {
        assert!(
            parsed.affected_names.iter().any(|name| name == longhand),
            "outline should affect {longhand}"
        );
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: longhand.to_owned(),
                value: "initial".to_owned(),
                priority: false,
            }),
            "{longhand} should be writable through PDB"
        );
    }
    assert!(
        style_entry_is_pdb_safe(&StyleEntry {
            name: "outline".to_owned(),
            value: "1px solid red".to_owned(),
            priority: true,
        }),
        "outline should no longer be a legacy side-table shorthand"
    );
    let outline_color_invert = StyleEntry {
        name: "outline-color".to_owned(),
        value: "invert".to_owned(),
        priority: false,
    };
    assert!(
        style_entry_is_pdb_safe(&outline_color_invert),
        "outline-color: invert should be accepted by the PDB write boundary"
    );
    assert!(
        style_entry_is_pdb_supplemental_side_entry(&outline_color_invert),
        "outline-color: invert should remain an explicit supplemental entry until Stylo owns it natively"
    );

    let state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block("outline: 1px solid red !important;"),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "outline").as_deref(),
        Some("red solid 1px"),
        "outline shorthand query should be served from PDB"
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "outline"),
        Some(true),
        "outline shorthand priority should be served from PDB"
    );

    let supplemental_invert =
        parse_style_property_entries_for_cssom_write("outline-color", "invert", false, None)
            .expect("outline-color: invert should stay accepted on the PDB supplemental path");
    assert_eq!(supplemental_invert.entries.len(), 1);
    assert_eq!(supplemental_invert.entries[0].name, "outline-color");
    assert_eq!(supplemental_invert.entries[0].value, "invert");
    assert!(!supplemental_invert.entries[0].priority);
    assert!(style_entry_is_pdb_supplemental_side_entry(
        &supplemental_invert.entries[0]
    ));

    let base_invert =
        parse_style_property_entries_with_base("outline-color", "invert", false, None)
            .expect("base parser should route outline-color: invert through PDB supplementals");
    assert_eq!(base_invert.entries.len(), 1);
    assert_eq!(base_invert.entries[0].name, "outline-color");
    assert_eq!(base_invert.entries[0].value, "invert");
    assert!(style_entry_is_pdb_supplemental_side_entry(
        &base_invert.entries[0]
    ));
    assert!(
        parse_style_property_entries_with_base("outline-color", "not-a-color", false, None)
            .is_none(),
        "base parser should not fall back to renderer value normalization for invalid outline-color"
    );
}

#[test]
fn css_ui_compat_longhands_use_pdb_for_unprefixed_entries() {
    for (property, value) in [
        ("appearance", "auto"),
        ("-webkit-appearance", "auto"),
        ("backface-visibility", "visible"),
        ("background-clip", "text"),
        ("background-origin", "content-box"),
        ("order", "2"),
        ("transform-style", "preserve-3d"),
        ("user-select", "none"),
        ("-webkit-user-select", "text"),
        ("color-adjust", "economy"),
        ("forced-color-adjust", "preserve-parent-color"),
        ("print-color-adjust", "exact"),
        ("-webkit-text-fill-color", "red"),
    ] {
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: property.to_owned(),
                value: value.to_owned(),
                priority: false,
            }),
            "{property} should be owned by Stylo/PDB"
        );
        let parsed = parse_style_property_entries_with_pdb(property, value, false)
            .unwrap_or_else(|| panic!("{property} should parse through PDB"));
        assert!(
            !parsed.entries.is_empty(),
            "{property} should produce PDB entries"
        );
    }

    let property = "-webkit-transform-origin";
    assert!(
        !style_entry_is_pdb_safe(&StyleEntry {
            name: property.to_owned(),
            value: "20px 30px".to_owned(),
            priority: false,
        }),
        "{property} stays on the prefixed compatibility side-table path"
    );
    assert!(
        !known_style_property("-moz-user-select"),
        "-moz-user-select should not stay exposed as a Chromium-compatible side-table property"
    );
    assert!(
        parse_style_property_entries_for_cssom_write("-moz-user-select", "none", false, None)
            .is_none(),
        "-moz-user-select CSSOM writes should be rejected"
    );
}

#[test]
fn webkit_transform_origin_side_entry_uses_pdb_validation_gate() {
    assert!(
        !style_entry_is_pdb_safe(&StyleEntry {
            name: "-webkit-transform-origin".to_owned(),
            value: "20px 30px".to_owned(),
            priority: false,
        }),
        "-webkit-transform-origin stays on the prefixed compatibility side-table path"
    );

    let parsed = parse_style_property_entries_for_cssom_write(
        "-webkit-transform-origin",
        "20px 30px",
        true,
        None,
    )
    .expect("valid transform-origin grammar should pass the compat gate");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].name, "-webkit-transform-origin");
    assert_eq!(parsed.entries[0].value, "20px 30px");
    assert!(parsed.entries[0].priority);

    assert!(
        parse_style_property_entries_for_cssom_write(
            "-webkit-transform-origin",
            "banana",
            false,
            None,
        )
        .is_none(),
        "invalid transform-origin grammar should not fall through to raw side-entry storage"
    );
}

#[test]
fn webkit_text_fill_color_is_owned_by_stylo_pdb() {
    assert!(
        style_entry_is_pdb_safe(&StyleEntry {
            name: "-webkit-text-fill-color".to_owned(),
            value: "red".to_owned(),
            priority: false,
        }),
        "-webkit-text-fill-color should be parsed and retained by Stylo/PDB"
    );

    let parsed = parse_style_property_entries_with_pdb("-webkit-text-fill-color", "red", true)
        .expect("valid color grammar should parse through PDB");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].name, "-webkit-text-fill-color");
    assert_eq!(parsed.entries[0].value, "red");
    assert!(parsed.entries[0].priority);

    let state = inline_style_declaration_state_from_entries(&parsed.entries);
    assert!(state.side_entries.is_empty());
    assert_eq!(
        state
            .block
            .property_value("-webkit-text-fill-color")
            .as_deref(),
        Some("red")
    );

    assert!(
        parse_style_property_entries_with_pdb("-webkit-text-fill-color", "not-a-color", false,)
            .is_none(),
        "invalid color grammar should be rejected by Stylo"
    );
}

#[test]
fn newly_enabled_chromium_properties_use_pdb_capability_routing() {
    for (name, value) in [
        ("-webkit-line-clamp", "2"),
        ("scroll-margin", "1px 2px"),
        ("offset", "none"),
        ("position-try", "--fallback"),
        ("font-synthesis", "weight style small-caps"),
        ("text-wrap", "wrap balance"),
    ] {
        let parsed = parse_style_property_entries_for_cssom_write(name, value, false, None)
            .unwrap_or_else(|| panic!("Stylo/PDB should accept {name}: {value}"));
        assert!(
            !parsed.entries.is_empty(),
            "PDB projection should retain entries for {name}: {value}"
        );
        let state = inline_style_declaration_state_from_entries(&parsed.entries);
        assert!(
            state.side_entries.is_empty(),
            "Stylo-owned property should not use renderer side entries: {name}"
        );
        assert!(
            !state
                .block
                .property_value(name)
                .unwrap_or_default()
                .is_empty(),
            "Stylo-owned property should serialize through PDB: {name}"
        );
    }
}

#[test]
fn lightweight_standard_property_candidates_parse_through_pdb() {
    for (property, value) in [
        ("aspect-ratio", "1 / 2"),
        ("baseline-shift", "super"),
        ("background-position", "left top"),
        ("background-repeat", "repeat-x"),
        ("border-bottom-color", "red"),
        ("border-bottom-style", "dashed"),
        ("border-left-color", "red"),
        ("border-left-style", "dashed"),
        ("border-right-color", "red"),
        ("border-right-style", "dashed"),
        ("border-top-color", "red"),
        ("border-top-style", "dashed"),
        ("border-block-end-color", "red"),
        ("border-block-start-color", "red"),
        ("border-inline-end-color", "red"),
        ("border-inline-start-color", "red"),
        ("direction", "rtl"),
        ("flex-flow", "column wrap"),
        ("grid-column-start", "span 2"),
        ("grid-column-end", "3"),
        ("justify-self", "safe center"),
        ("perspective", "12px"),
        ("place-content", "center start"),
        ("reading-flow", "grid-order"),
        ("reading-order", "-2"),
        ("word-spacing", "2px"),
        ("writing-mode", "vertical-rl"),
    ] {
        let parsed = parse_style_property_entries_for_cssom_write(property, value, false, None)
            .unwrap_or_else(|| panic!("{property}: {value} should parse through PDB"));
        assert!(
            !parsed.entries.is_empty(),
            "{property}: {value} should produce PDB entries"
        );
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: property.to_owned(),
                value: value.to_owned(),
                priority: false,
            }),
            "{property}: {value} should be PDB-safe"
        );
    }

    for (property, value) in [
        ("border-block-start-color", "not-a-color"),
        ("border-inline-end-color", "not-a-color"),
        ("direction", "sideways"),
        ("reading-flow", "auto"),
        ("reading-order", "1.5"),
        ("writing-mode", "horizontal"),
    ] {
        assert!(
            parse_style_property_entries_for_cssom_write(property, value, false, None).is_none(),
            "{property}: {value} should be rejected by the PDB write boundary"
        );
    }
}

#[test]
fn inline_state_builder_keeps_legacy_shorthands_out_of_pdb_entries() {
    let border = style_entry("border", "1px solid black");
    let border_top = style_entry("border-top", "1px solid black");
    let border_width = style_entry("border-width", "1px");
    let border_radius = style_entry("border-radius", "1px 2px");
    let outline = style_entry("outline", "1px solid red");
    let padding_left = style_entry("padding-left", "1px");
    let text_decoration = style_entry("text-decoration", "overline from-font dotted green");
    let text_emphasis = style_entry("text-emphasis", "dot red");
    let webkit_text_stroke = style_entry("-webkit-text-stroke", "1px red");

    assert!(
        style_entry_is_pdb_safe(&border),
        "border shorthand is now owned by Stylo/PDB"
    );
    assert!(
        style_entry_is_pdb_safe(&border_width),
        "border component shorthands are now owned by Stylo/PDB"
    );
    assert!(
        style_entry_is_pdb_safe(&border_top),
        "border side shorthands are now owned by Stylo/PDB"
    );
    assert!(
        style_entry_is_pdb_safe(&border_radius),
        "border-radius shorthand is now owned by Stylo/PDB"
    );
    assert!(
        style_entry_is_pdb_safe(&outline),
        "outline shorthand is now owned by Stylo/PDB"
    );
    assert!(
        style_entry_is_pdb_safe(&text_decoration),
        "text-decoration shorthand is now owned by Stylo/PDB"
    );
    assert!(
        style_entry_is_pdb_safe(&text_emphasis),
        "text-emphasis shorthand is now owned by Stylo/PDB"
    );
    assert!(
        style_entry_is_pdb_safe(&style_entry("font-variant", "small-caps")),
        "font-variant shorthand is now owned by Stylo/PDB"
    );
    assert!(
        style_entry_is_pdb_safe(&webkit_text_stroke),
        "-webkit-text-stroke shorthand is now owned by Stylo/PDB"
    );
    assert!(
        inline_style_entry_is_pdb_storage_candidate(&padding_left),
        "inline PDB state queries still need to recognize direct-storage box longhands"
    );

    let state = inline_style_declaration_state_from_entries(std::slice::from_ref(&border));
    assert!(!state.block.is_empty());
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].name, border.name);
    assert_eq!(state.entries[0].value, border.value);
    assert_eq!(state.entries[0].priority, border.priority);
    assert!(state.side_entries.is_empty());
    assert_eq!(
        state.block.property_value("border").as_deref(),
        Some("1px solid black")
    );
}

#[test]
fn inline_serialized_entries_seed_pdb_state_without_css_text_reparse() {
    assert!(inline_style_entry_is_pdb_storage_candidate(&style_entry(
        "display", "block",
    )));
    assert!(inline_style_entry_is_pdb_storage_candidate(&style_entry(
        "visibility",
        "hidden",
    )));
    let plain = inline_style_declaration_state_from_serialized_entries(
        &[
            style_entry("display", "block"),
            StyleEntry {
                priority: true,
                ..style_entry("visibility", "hidden")
            },
        ],
        "display: block; visibility: hidden !important;",
        None,
    );
    assert!(
        plain.entries.is_empty(),
        "pure PDB inline state should not keep ordinary adapter entries"
    );
    assert!(plain.side_entries.is_empty());
    assert_eq!(
        plain.block.property_value("display").as_deref(),
        Some("block")
    );
    assert!(plain.block.property_priority("visibility"));
    assert_eq!(
        plain.css_text(),
        "display: block; visibility: hidden !important;"
    );

    let mixed = inline_style_declaration_state_from_serialized_entries(
        &[
            style_entry("--token", "value"),
            style_entry("visibility", "hidden"),
            style_entry("background-image", r#"url("https://example.test/img.png")"#),
        ],
        r#"--token: value; visibility: hidden; background-image: url("https://example.test/img.png");"#,
        None,
    );
    assert!(mixed.side_entries.is_empty());
    assert!(
        mixed
            .block
            .entries()
            .iter()
            .any(|entry| entry.name == "--token")
    );
    assert_eq!(
        mixed.block.property_value("--token").as_deref(),
        Some("value")
    );
    assert_eq!(
        mixed.block.property_value("visibility").as_deref(),
        Some("hidden")
    );
    assert_eq!(
        mixed.block.property_value("background-image").as_deref(),
        Some(r#"url("https://example.test/img.png")"#)
    );
    assert_eq!(
        mixed
            .entries()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["--token", "visibility", "background-image"]
    );

    let border_entry = style_entry("border", "calc(10px) solid pink");
    assert!(
        inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(
            std::slice::from_ref(&border_entry)
        )
    );
    let border_shorthand = inline_style_declaration_state_from_serialized_entries(
        std::slice::from_ref(&border_entry),
        "border: calc(10px) solid pink;",
        None,
    );
    assert!(border_shorthand.entries.is_empty());
    assert!(border_shorthand.side_entries.is_empty());
    assert_eq!(
        border_shorthand.block.property_value("border").as_deref(),
        Some("calc(10px) solid pink")
    );

    let border_side_entries = [style_entry("border-top", "calc(11px) solid pink")];
    assert!(
        inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(&border_side_entries)
    );
    let border_side_shorthand = inline_style_declaration_state_from_serialized_entries(
        &border_side_entries,
        "border-top: calc(11px) solid pink;",
        None,
    );
    assert!(!border_side_shorthand.block.is_empty());
    assert!(border_side_shorthand.entries.is_empty());
    assert!(border_side_shorthand.side_entries.is_empty());
    assert_eq!(
        border_side_shorthand
            .block
            .property_value("border-top")
            .as_deref(),
        Some("calc(11px) solid pink")
    );
    assert_eq!(
        border_side_shorthand
            .entries()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["border-top-width", "border-top-style", "border-top-color"]
    );
    assert_eq!(
        border_side_shorthand.css_text(),
        "border-top: calc(11px) solid pink;"
    );
}

#[test]
fn inline_unresolved_and_custom_values_respect_pdb_compat_boundaries() {
    let entries = [
        style_entry("--token", "var(--fallback, value)"),
        style_entry("padding", "var(--pad)"),
        style_entry("width", "env(safe-area-inset-top)"),
    ];
    assert!(
        style_entry_is_pdb_safe(&entries[0]),
        "non-empty custom properties should be owned by Stylo/PDB"
    );
    assert!(
        style_entry_is_pdb_safe(&style_entry("--empty", " ")),
        "whitespace custom property specified values should be owned by Stylo/PDB"
    );
    let parsed_empty_custom =
        parse_style_property_entries_for_cssom_write("--empty", "  ", false, None)
            .expect("whitespace custom property values should stay accepted");
    assert_eq!(parsed_empty_custom.entries[0].value, "");
    assert!(style_entry_is_pdb_safe(&parsed_empty_custom.entries[0]));
    let parsed_custom =
        parse_style_property_entries_with_pdb("--token", "var(--fallback, value)", true)
            .expect("non-empty custom property should parse through Stylo/PDB");
    assert_eq!(
        parsed_custom
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
            .collect::<Vec<_>>(),
        [("--token", "var(--fallback, value)", true)]
    );
    assert!(
        style_entry_is_pdb_safe(&entries[1]),
        "box shorthand unresolved values use PDB plus renderer order projection"
    );
    assert!(
        style_entry_is_pdb_safe(&entries[2]),
        "ordinary unresolved values accepted by Stylo are owned by PDB"
    );
    assert!(
        parse_style_property_entries_for_cssom_write("width", "var(--x ())", false, None).is_none(),
        "invalid var() syntax must still be rejected by the PDB value-fragment parser"
    );
    assert!(
        cssom_style_property_write_uses_pdb("top", "env(test 0 1, green)"),
        "env() indexed syntax should use PDB once Stylo accepts it"
    );
    let parsed_indexed_env =
        parse_style_property_entries_with_pdb("top", "env(test 0 1, green)", false)
            .expect("indexed env() should parse through Stylo/PDB");
    assert_eq!(
        parsed_indexed_env
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [("top", "env(test 0 1, green)")]
    );
    let parsed_padding = parse_style_property_entries_with_pdb("padding", "var(--pad)", false)
        .expect("padding var() should parse through Stylo/PDB");
    assert_eq!(
        parsed_padding
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [("padding", "var(--pad)")]
    );

    let state = inline_style_declaration_state_from_serialized_entries(
        &entries,
        "--token: var(--fallback, value); padding: var(--pad); width: env(safe-area-inset-top);",
        None,
    );
    assert!(state.side_entries.is_empty());
    assert!(
        state
            .block
            .entries()
            .iter()
            .any(|entry| entry.name == "--token")
    );
    assert_eq!(
        state.block.property_value("--token").as_deref(),
        Some("var(--fallback, value)")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "--token").as_deref(),
        Some("var(--fallback, value)")
    );
    assert_eq!(
        state.block.property_value("padding").as_deref(),
        Some("var(--pad)")
    );
    assert_eq!(
        state.block.property_value("width").as_deref(),
        Some("env(safe-area-inset-top)")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "width").as_deref(),
        Some("env(safe-area-inset-top)")
    );
    assert_eq!(
        state.css_text(),
        "--token: var(--fallback, value); padding: var(--pad); width: env(safe-area-inset-top);"
    );

    let empty_custom_state =
        inline_style_declaration_state_from_css_text("--empty:; --space:  ;", None);
    assert!(empty_custom_state.side_entries.is_empty());
    assert!(empty_custom_state.block.property_is_declared("--empty"));
    assert!(empty_custom_state.block.property_is_declared("--space"));
    assert_eq!(
        inline_state_property_value_with_pdb(&empty_custom_state, "--empty").as_deref(),
        Some(" ")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&empty_custom_state, "--space").as_deref(),
        Some(" ")
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&empty_custom_state, "--empty"),
        Some(false)
    );
    assert_eq!(empty_custom_state.css_text(), "--empty: ; --space: ;");

    assert!(inline_css_text_pdb_storage_state("margin: var(--prop); margin-top: 10px").is_none());
    assert_eq!(
            inline_style_declaration_state_from_css_text(
                "margin: var(--prop); margin-top: 10px",
                None,
            )
            .css_text(),
            "margin-right: ; margin-bottom: ; margin-left: ; margin-top: 10px;"
        );
    assert!(
        inline_css_text_pdb_storage_state("border-width: var(--width); border-left-width: 3px")
            .is_none()
    );
    assert_eq!(
        inline_style_declaration_state_from_css_text(
            "border-width: var(--width); border-left-width: 3px",
            None,
        )
        .css_text(),
        "border-top-width: ; border-right-width: ; border-bottom-width: ; border-left-width: 3px;"
    );
}

#[test]
fn unresolved_pdb_shorthand_entries_expand_before_longhand_mutation() {
    for (shorthand, mutation_longhand) in [
        ("padding-block", "padding-block-start"),
        ("padding-inline", "padding-inline-end"),
        ("overflow", "overflow-x"),
        ("outline", "outline-color"),
        ("text-decoration", "text-decoration-line"),
        ("text-emphasis", "text-emphasis-color"),
        ("font-variant", "font-variant-caps"),
        ("transition", "transition-duration"),
        ("animation", "animation-name"),
        ("font", "font-size"),
        ("background", "background-color"),
        ("gap", "column-gap"),
        ("place-content", "justify-content"),
    ] {
        let longhands = unresolved_box_shorthand_longhands(shorthand)
            .unwrap_or_else(|| panic!("{shorthand} should expand unresolved storage"));
        assert!(
            longhands.contains(&mutation_longhand),
            "{shorthand} should cover {mutation_longhand}"
        );
        let affected_names = style_property_mutation_affected_names_with_pdb(mutation_longhand)
            .unwrap_or_else(|| panic!("{mutation_longhand} should be PDB-backed"));
        let mut entries = vec![StyleEntry {
            name: shorthand.to_owned(),
            value: "var(--token)".to_owned(),
            priority: true,
        }];

        expand_unresolved_box_shorthand_entries_for_mutation(&mut entries, &affected_names);

        assert!(
            !entries.iter().any(|entry| entry.name == shorthand),
            "{shorthand} should not remain next to a mutated longhand"
        );
        for longhand in longhands {
            let should_keep = !affected_names.iter().any(|affected| affected == longhand);
            assert_eq!(
                entries.iter().any(|entry| {
                    entry.name == *longhand && entry.value.is_empty() && entry.priority
                }),
                should_keep,
                "{shorthand} placeholder state for {longhand}"
            );
        }
    }
}

#[test]
fn inline_pdb_mutation_keeps_unresolved_shorthand_projection() {
    let mut state = CssInlineStyleDeclarationState::default();
    let affected_names = style_property_affected_names_with_pdb("padding").unwrap();
    let parsed = parse_style_property_entries_with_pdb("padding", "var(--pad)", false)
        .expect("padding var() should parse through Stylo/PDB");
    let projection = state
        .block
        .set_property_with_projection("padding", "var(--pad)", false);
    assert_ne!(
        projection.set_result,
        moli_css_parse::CssSetResult::ParseError
    );
    let mut entries = projection
        .entries
        .into_iter()
        .map(StyleEntry::from)
        .collect::<Vec<_>>();
    if entries.is_empty() || entries.iter().any(|entry| entry.value.is_empty()) {
        entries = parsed.entries.clone();
    }
    refresh_inline_state_entries_after_pdb_mutation(
        &mut state,
        "padding",
        &affected_names,
        entries,
        Vec::new(),
    );

    assert_eq!(
        state.property_names(),
        [
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left"
        ]
    );
    assert_eq!(state.css_text(), "padding: var(--pad);");
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "padding").as_deref(),
        Some("var(--pad)")
    );

    let affected_names = style_property_affected_names_with_pdb("padding-left").unwrap();
    let parsed = parse_style_property_entries_with_pdb("padding-left", "calc(calc(1px))", false)
        .expect("padding-left calc() should parse through Stylo/PDB");
    assert_eq!(
        parsed
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [("padding-left", "calc(1px)")]
    );
    let projection =
        state
            .block
            .set_property_with_projection("padding-left", "calc(calc(1px))", false);
    assert_ne!(
        projection.set_result,
        moli_css_parse::CssSetResult::ParseError
    );
    let entries = projection
        .entries
        .into_iter()
        .map(StyleEntry::from)
        .collect::<Vec<_>>();
    refresh_inline_state_entries_after_pdb_mutation(
        &mut state,
        "padding-left",
        &affected_names,
        entries,
        Vec::new(),
    );
    assert_eq!(
        state
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [
            ("padding-top", ""),
            ("padding-right", ""),
            ("padding-bottom", ""),
            ("padding-left", "calc(1px)")
        ]
    );
    assert_eq!(
        state.property_names(),
        [
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left"
        ]
    );
    assert_eq!(
        state.css_text(),
        "padding-top: ; padding-right: ; padding-bottom: ; padding-left: calc(1px);"
    );
    assert_eq!(
        state.style_resolution_text(),
        "padding: var(--pad) var(--pad) var(--pad) calc(1px);"
    );

    let affected_names = style_property_affected_names_with_pdb("padding-left").unwrap();
    let parsed = parse_style_property_entries_with_pdb("padding-left", "2px", true)
        .expect("padding-left important value should parse through Stylo/PDB");
    let entries = set_pdb_block_property_collecting_entries(
        &mut state.block,
        "padding-left",
        "2px",
        true,
        &parsed,
        false,
    )
    .expect("padding-left important value should update PDB");
    refresh_inline_state_entries_after_pdb_mutation(
        &mut state,
        "padding-left",
        &affected_names,
        entries,
        Vec::new(),
    );
    assert_eq!(
        state.css_text(),
        "padding-top: ; padding-right: ; padding-bottom: ; padding-left: 2px !important;"
    );

    let removed = state.block.remove_property("padding-left");
    assert!(removed.changed);
    refresh_inline_state_entries_after_pdb_mutation(
        &mut state,
        "padding-left",
        &affected_names,
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(
        state.css_text(),
        "padding-top: ; padding-right: ; padding-bottom: ;"
    );
}

#[test]
fn inline_pdb_mutation_materializes_existing_block_before_renderer_projection() {
    let mut state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block("font-size: unset"),
        ..Default::default()
    };
    assert!(state.entries.is_empty());
    assert_eq!(state.css_text(), "font-size: unset;");

    let affected_names = style_property_affected_names_with_pdb("margin-bottom").unwrap();
    let parsed = parse_style_property_entries_with_pdb("margin-bottom", "var(--x)", false)
        .expect("margin-bottom var() should parse through Stylo/PDB");
    let mut entries = set_pdb_block_property_collecting_entries(
        &mut state.block,
        "margin-bottom",
        "var(--x)",
        false,
        &parsed,
        false,
    )
    .expect("margin-bottom var() should update the PDB block");
    if entries.is_empty() || entries.iter().any(|entry| entry.value.is_empty()) {
        entries = parsed.entries.clone();
    }

    refresh_inline_state_entries_after_pdb_mutation(
        &mut state,
        "margin-bottom",
        &affected_names,
        entries,
        Vec::new(),
    );

    assert_eq!(
        state
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        [("font-size", "unset"), ("margin-bottom", "var(--x)")]
    );
    assert_eq!(
        state.css_text(),
        "font-size: unset; margin-bottom: var(--x);"
    );
    assert_eq!(
        state.style_resolution_text(),
        "font-size: unset; margin-bottom: var(--x);"
    );
}

#[test]
fn inline_serialized_border_state_keeps_pdb_query_with_side_entries() {
    let css_text =
        "border: 1px solid red !important; --token: value; -webkit-transform-origin: 20px 30px;";
    let mut entries = parse_style_property_entries_with_pdb("border", "1px solid red", true)
        .expect("border should parse through PDB")
        .entries;
    entries.push(style_entry("--token", "value"));
    entries.push(style_entry("-webkit-transform-origin", "20px 30px"));
    let state = inline_style_declaration_state_from_serialized_entries(&entries, css_text, None);

    assert_eq!(
        state
            .side_entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["-webkit-transform-origin"]
    );
    assert!(
        state
            .block
            .entries()
            .iter()
            .any(|entry| entry.name == "--token")
    );
    assert_eq!(
        state.block.property_value("--token").as_deref(),
        Some("value")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "border").as_deref(),
        Some("1px solid red")
    );
    assert_eq!(
        inline_state_property_priority_with_pdb(&state, "border"),
        Some(true)
    );
}

#[test]
fn structured_properties_use_pdb_entries() {
    for (name, value) in [
        ("appearance", "auto"),
        ("background-image", r#"url("https://example.test/img.png")"#),
        ("background-blend-mode", "multiply"),
        ("color-adjust", "exact"),
        ("color-scheme", "dark only"),
        ("column-rule-width", "0"),
        ("column-width", "0"),
        ("content", "\"x\""),
        ("forced-color-adjust", "preserve-parent-color"),
        ("isolation", "isolate"),
        ("mix-blend-mode", "multiply"),
        ("orphans", "2"),
        ("overscroll-behavior-block", "contain"),
        ("overscroll-behavior-inline", "none"),
        ("overscroll-behavior-x", "contain"),
        ("overscroll-behavior-y", "none"),
        ("print-color-adjust", "exact"),
        ("quotes", "\"a\" \"b\""),
        ("scrollbar-color", "auto"),
        ("scrollbar-width", "thin"),
        ("scroll-margin-top", "0"),
        ("scroll-padding-bottom", "0"),
        ("scroll-snap-align", "start start"),
        ("shape-margin", "0"),
        ("text-shadow", "red 1px 2px 3px"),
        ("text-underline-offset", "1px"),
        ("text-underline-position", "under"),
        ("user-select", "none"),
        ("will-change", "transform"),
        ("widows", "3"),
        ("zoom", "1.5"),
    ] {
        let parsed = parse_style_property_entries_for_cssom_write(name, value, false, None)
            .unwrap_or_else(|| panic!("{name} should parse through PDB"));
        assert_eq!(parsed.entries.len(), 1);
        let expected_name = if name == "color-adjust" {
            "print-color-adjust"
        } else {
            name
        };
        assert_eq!(parsed.entries[0].name, expected_name);
        assert!(parse_style_property_entries_with_pdb(name, value, false).is_some());
        assert!(style_entry_is_pdb_safe(&StyleEntry {
            name: expected_name.to_owned(),
            value: parsed.entries[0].value.clone(),
            priority: false,
        }));
    }

    let snap = parse_style_property_entries_for_cssom_write(
        "scroll-snap-align",
        "start invalid",
        false,
        None,
    );
    assert!(snap.is_none());
}

#[test]
fn legacy_structured_longhands_use_pdb_when_stylo_owns_the_property() {
    for (name, value, expected_name, expected_value) in [
        ("color-scheme", "dark only", "color-scheme", "dark only"),
        ("orphans", "2", "orphans", "2"),
        ("widows", "3", "widows", "3"),
        ("page-break-after", "always", "break-after", "page"),
        ("page-break-before", "avoid", "break-before", "avoid"),
        ("page-break-inside", "avoid", "break-inside", "avoid"),
    ] {
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: name.to_owned(),
                value: value.to_owned(),
                priority: false,
            }),
            "{name} should be owned by Stylo/PDB"
        );
        let parsed = parse_style_property_entries_for_cssom_write(name, value, false, None)
            .unwrap_or_else(|| panic!("{name} should parse through PDB"));
        assert!(
            parsed
                .entries
                .iter()
                .any(|entry| entry.name == expected_name && entry.value == expected_value),
            "{name} should produce {expected_name}: {expected_value}, got {:?}",
            parsed
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>()
        );
    }

    let state = CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block(
            "color-scheme: dark only; orphans: 2; widows: 3; \
                 page-break-after: always; page-break-before: left; \
                 page-break-inside: avoid;",
        ),
        ..Default::default()
    };
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "color-scheme").as_deref(),
        Some("dark only")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "orphans").as_deref(),
        Some("2")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "widows").as_deref(),
        Some("3")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "page-break-after").as_deref(),
        Some("always")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "page-break-before").as_deref(),
        Some("left")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "page-break-inside").as_deref(),
        Some("avoid")
    );
}

#[test]
fn overscroll_behavior_family_uses_pdb_entries_and_cssom_folding() {
    let parsed = parse_style_property_entries_for_cssom_write(
        "overscroll-behavior",
        "contain none",
        true,
        None,
    )
    .expect("overscroll-behavior shorthand should parse through PDB");
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries[0].name, "overscroll-behavior-x");
    assert_eq!(parsed.entries[0].value, "contain");
    assert!(parsed.entries[0].priority);
    assert_eq!(parsed.entries[1].name, "overscroll-behavior-y");
    assert_eq!(parsed.entries[1].value, "none");
    assert!(parsed.entries[1].priority);
    assert!(
        parse_style_property_entries_with_pdb("overscroll-behavior", "contain none", true,)
            .is_some()
    );
    assert!(style_entry_is_pdb_safe(&StyleEntry {
        name: "overscroll-behavior".to_owned(),
        value: "contain none".to_owned(),
        priority: true,
    }));

    let state = inline_style_declaration_state_from_entries(&[
        StyleEntry {
            name: "overscroll-behavior-x".to_owned(),
            value: "contain".to_owned(),
            priority: false,
        },
        StyleEntry {
            name: "overscroll-behavior-y".to_owned(),
            value: "contain".to_owned(),
            priority: false,
        },
    ]);
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "overscroll-behavior").as_deref(),
        Some("contain")
    );
    assert_eq!(
        inline_state_property_value_with_pdb(&state, "overscroll-behavior-x").as_deref(),
        Some("contain")
    );
}

#[test]
fn overscroll_behavior_logical_longhands_use_pdb_entries() {
    for (name, value) in [
        ("overscroll-behavior-block", "contain"),
        ("overscroll-behavior-inline", "none"),
    ] {
        let parsed = parse_style_property_entries_for_cssom_write(name, value, true, None)
            .unwrap_or_else(|| panic!("{name} should parse through PDB"));
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, name);
        assert_eq!(parsed.entries[0].value, value);
        assert!(parsed.entries[0].priority);
        assert_eq!(parsed.affected_names, vec![name.to_owned()]);
        assert!(parse_style_property_entries_with_pdb(name, value, true).is_some());
        assert!(style_entry_is_pdb_safe(&StyleEntry {
            name: name.to_owned(),
            value: value.to_owned(),
            priority: true,
        }));

        let state = inline_style_declaration_state_from_entries(&[StyleEntry {
            name: name.to_owned(),
            value: value.to_owned(),
            priority: true,
        }]);
        assert_eq!(
            inline_state_property_value_with_pdb(&state, name).as_deref(),
            Some(value)
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, name),
            Some(true)
        );
    }
}

#[test]
fn text_shadow_cssom_write_uses_pdb_serialization() {
    let parsed =
        parse_style_property_entries_for_cssom_write("text-shadow", "1px 2px 3px red", false, None)
            .expect("text-shadow should parse through PDB");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].name, "text-shadow");
    assert_eq!(parsed.entries[0].value, "red 1px 2px 3px");
    assert!(
        parse_style_property_entries_with_pdb("text-shadow", "1px 2px 3px red", false,).is_some()
    );
    assert!(style_entry_is_pdb_safe(&parsed.entries[0]));

    let inline =
        parse_style_property_entries_with_base("text-shadow", "1px 2px 3px red", false, None)
            .expect("text-shadow should parse through the strict Stylo path");
    assert_eq!(inline.entries.len(), 1);
    assert_eq!(inline.entries[0].name, "text-shadow");
    assert_eq!(inline.entries[0].value, parsed.entries[0].value);
}

#[test]
fn background_image_image_set_uses_pdb_storage() {
    let value = r#"image-set(url("") calc(1x * NaN))"#;
    let parsed =
        parse_style_property_entries_for_cssom_write("background-image", value, false, None)
            .expect("valid image-set resolution math should parse");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].name, "background-image");
    assert_eq!(
        parsed.entries[0].value,
        r#"image-set(url("") calc(NaN * 1dppx))"#
    );
    assert!(
        cssom_style_property_write_uses_pdb("background-image", value),
        "image-set should use PDB once Stylo mutation exposes the same CSSOM surface"
    );
    assert!(
        style_entry_is_pdb_safe(&parsed.entries[0]),
        "image-set should be promoted to PDB storage after equivalent serialization is proven"
    );
    let pdb = parse_style_property_entries_with_pdb("background-image", value, false)
        .expect("image-set should parse through PDB");
    assert_eq!(pdb.entries.len(), parsed.entries.len());
    assert_eq!(pdb.entries[0].name, parsed.entries[0].name);
    assert_eq!(pdb.entries[0].value, parsed.entries[0].value);
    assert_eq!(pdb.entries[0].priority, parsed.entries[0].priority);
}

#[test]
fn zoom_uses_pdb_for_cssom_compatible_values() {
    for value in ["normal", "100%", "0", "calc(1 - 0.5)"] {
        let parsed = parse_style_property_entries_for_cssom_write("zoom", value, false, None)
            .unwrap_or_else(|| panic!("zoom: {value} should parse through PDB"));
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "zoom");
        let stylo = stylo_pdb_entries_for_property("zoom", value, false)
            .unwrap_or_else(|| panic!("zoom: {value} should be accepted by Stylo/PDB"));
        assert_eq!(stylo.entries.len(), 1);
        assert_eq!(stylo.entries[0].name, "zoom");
        assert_eq!(stylo.entries[0].value, parsed.entries[0].value);
        assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
        assert!(
            !style_entry_is_pdb_supplemental_side_entry(&parsed.entries[0]),
            "zoom: {value} should stay only in the PDB block"
        );
    }

    let dynamic = parse_style_property_entries_for_cssom_write(
        "zoom",
        "calc(sign(1em - 1px) * 2%)",
        false,
        None,
    )
    .expect("dynamic zoom should stay accepted for CSSOM compat");
    assert_eq!(dynamic.entries.len(), 1);
    assert_eq!(dynamic.entries[0].name, "zoom");
    assert_eq!(dynamic.entries[0].value, "calc(2% * sign(1em - 1px))");
    assert!(!style_entry_is_pdb_supplemental_side_entry(
        &dynamic.entries[0]
    ));
}

#[test]
fn grid_column_shorthand_uses_pdb_entries() {
    let parsed = parse_style_property_entries_for_cssom_write("grid-column", "1 / 3", true, None)
        .expect("grid-column should parse through PDB");
    assert_eq!(parsed.affected_names[0], "grid-column");
    assert!(
        parsed
            .affected_names
            .contains(&"grid-column-start".to_owned())
    );
    assert!(
        parsed
            .affected_names
            .contains(&"grid-column-end".to_owned())
    );
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries[0].name, "grid-column-start");
    assert_eq!(parsed.entries[0].value, "1");
    assert!(parsed.entries[0].priority);
    assert_eq!(parsed.entries[1].name, "grid-column-end");
    assert_eq!(parsed.entries[1].value, "3");
    assert!(parsed.entries[1].priority);
    assert!(parse_style_property_entries_with_pdb("grid-column", "1 / 3", true).is_some());
}

#[test]
fn list_style_shorthand_uses_pdb_entries() {
    let parsed =
        parse_style_property_entries_for_cssom_write("list-style", "inside disc", false, None)
            .expect("list-style should parse through PDB");
    assert_eq!(parsed.affected_names[0], "list-style");
    assert_eq!(parsed.entries.len(), 3);
    assert_eq!(parsed.entries[0].name, "list-style-position");
    assert_eq!(parsed.entries[0].value, "inside");
    assert_eq!(parsed.entries[1].name, "list-style-image");
    assert_eq!(parsed.entries[1].value, "none");
    assert_eq!(parsed.entries[2].name, "list-style-type");
    assert_eq!(parsed.entries[2].value, "disc");
    assert!(parse_style_property_entries_with_pdb("list-style", "inside disc", false).is_some());
}

#[test]
fn remaining_structured_longhands_use_pdb_entries() {
    for (name, input, expected) in [
        ("alignment-baseline", "alphabetic", "alphabetic"),
        ("background-attachment", "local", "local"),
        ("baseline-source", "first", "first"),
        ("bookmark-level", "1", "1"),
        ("bookmark-state", "closed", "closed"),
        ("border-collapse", "collapse", "collapse"),
        ("caption-side", "bottom", "bottom"),
        ("clear", "both", "both"),
        (
            "clip",
            "rect(0px, 1px, 2px, 3px)",
            "rect(0px, 1px, 2px, 3px)",
        ),
        ("empty-cells", "hide", "hide"),
        (
            "link-parameters",
            "param(--a, orange), param(--b)",
            "param(--a, orange), param(--b)",
        ),
        ("list-style-position", "inside", "inside"),
        ("list-style-type", "upper-alpha", "upper-alpha"),
        ("table-layout", "fixed", "fixed"),
        (
            "text-size-adjust",
            "calc(10% * sibling-index())",
            "calc(10% * sibling-index())",
        ),
        ("text-transform", "uppercase", "uppercase"),
    ] {
        assert!(
            cssom_style_property_write_uses_pdb(name, input),
            "{name} should be PDB-backed for CSSOM writes"
        );
        let parsed = parse_style_property_entries_for_cssom_write(name, input, true, None)
            .unwrap_or_else(|| panic!("{name}: {input} should parse through PDB"));
        assert_eq!(parsed.entries.len(), 1, "{name}: {input}");
        assert_eq!(parsed.entries[0].name, name, "{name}: {input}");
        assert_eq!(parsed.entries[0].value, expected, "{name}: {input}");
        assert!(parsed.entries[0].priority, "{name}: {input}");
        assert_eq!(parsed.affected_names, vec![name.to_owned()]);
        assert!(
            parse_style_property_entries_with_pdb(name, input, true).is_some(),
            "{name}: {input} should parse directly through PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&parsed.entries[0]),
            "{name}: {input} should stay PDB-safe"
        );
    }

    let text_size = parse_style_property_entries_for_cssom_write(
        "text-size-adjust",
        "calc(10% + 5%)",
        false,
        None,
    )
    .expect("static text-size-adjust calc should parse through PDB");
    assert_eq!(text_size.entries[0].value, "calc(15%)");

    let link_empty = parse_style_property_entries_for_cssom_write(
        "link-parameters",
        "param(--a, )",
        false,
        None,
    )
    .expect("empty link-parameters fallback should parse through PDB");
    assert_eq!(link_empty.entries[0].value, "param(--a, )");

    let link_eof =
        parse_style_property_entries_for_cssom_write("link-parameters", "param(--a", false, None)
            .expect("EOF-recovered link-parameters function should parse through PDB");
    assert_eq!(link_eof.entries[0].value, "param(--a)");

    assert!(
        parse_style_property_entries_for_cssom_write("color", "red; width: 1px", false, None)
            .is_none(),
        "CSSOM value fragments must not be parsed as declaration source"
    );
}

#[test]
fn base_fallback_routes_structured_pdb_properties_through_pdb() {
    fn assert_base_matches_pdb(property: &str, value: &str) {
        let base = parse_style_property_entries_with_base(property, value, true, None)
            .unwrap_or_else(|| panic!("{property}: {value} should parse through base fallback"));
        let direct = parse_style_property_entries_with_pdb(property, value, true)
            .unwrap_or_else(|| panic!("{property}: {value} should parse directly through PDB"));
        assert_eq!(
            base.affected_names, direct.affected_names,
            "{property}: {value} should use PDB affected names in base fallback"
        );
        assert_eq!(
            base.entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
                .collect::<Vec<_>>(),
            direct
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
                .collect::<Vec<_>>(),
            "{property}: {value} base fallback output should match direct PDB output"
        );
        assert!(
            base.entries.iter().all(style_entry_is_pdb_safe),
            "{property}: {value} base fallback entries should stay PDB-safe"
        );
    }

    for (property, value) in [
        ("align-content", "first baseline"),
        ("align-items", "first baseline"),
        ("align-self", "first baseline"),
        ("background-size", "calc(10px + 5px) 20px"),
        ("color", "rgb(0 128 0 / 50%)"),
        ("color-scheme", "dark only"),
        ("column-rule-width", "0"),
        ("column-width", "0"),
        ("content", "'string'"),
        ("gap", "10px 10px"),
        ("grid-column", "1 / 3"),
        ("justify-self", "safe center"),
        ("link-parameters", "param(--a"),
        ("list-style", "inside disc"),
        ("orphans", "2"),
        ("overscroll-behavior", "chain chain"),
        ("page-break-after", "always"),
        ("place-content", "center center"),
        ("scroll-margin-top", "0"),
        ("scroll-padding-bottom", "0"),
        ("scroll-snap-align", "start start"),
        ("shape-margin", "0"),
        ("text-shadow", "1px 2px 3px red"),
        ("text-size-adjust", "calc(10% + 5%)"),
        ("will-change", "transform"),
        ("widows", "3"),
        ("width", "calc(10px + 1vmin + 10%)"),
        ("zoom", "calc(1 - 0.5)"),
    ] {
        assert_base_matches_pdb(property, value);
    }

    for (property, value) in [
        ("color", "red; width: 1px"),
        ("column-rule-width", "-1px"),
        ("column-width", "-1px"),
        ("link-parameters", "param(-a)"),
        ("scroll-padding-bottom", "-1px"),
        ("scroll-snap-align", "start invalid"),
        ("shape-margin", "-1px"),
        ("text-size-adjust", "10px"),
        ("width", "calc(5px / 1px)"),
    ] {
        assert!(
            parse_style_property_entries_with_base(property, value, false, None).is_none(),
            "{property}: {value} should be rejected by the PDB-backed base fallback"
        );
    }
}

#[test]
fn remaining_structured_longhands_reject_invalid_values_with_pdb() {
    for (name, value) in [
        ("bookmark-level", "0"),
        ("bookmark-state", "none"),
        ("text-size-adjust", "-100%"),
        ("text-size-adjust", "10px"),
        ("link-parameters", "param(-a)"),
        ("link-parameters", "param(--a red)"),
        ("link-parameters", "param(--a, red) param(--b, blue)"),
    ] {
        assert!(
            parse_style_property_entries_for_cssom_write(name, value, false, None).is_none(),
            "{name}: {value} should be rejected by CSSOM write parsing"
        );
        assert!(
            parse_style_property_entries_with_pdb(name, value, false).is_none(),
            "{name}: {value} should be rejected by direct PDB parsing"
        );
    }
}

#[test]
fn overflow_overlay_uses_pdb_supplemental_cssom_path() {
    assert!(parse_style_property_entries_with_base("overflow", "banana", false, None).is_none());

    let parsed = parse_style_property_entries_for_cssom_write("overflow-x", "overlay", false, None)
        .expect("overflow overlay should remain accepted for CSSOM compat");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].name, "overflow-x");
    assert_eq!(parsed.entries[0].value, "overlay");
    assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
    assert!(style_entry_is_pdb_supplemental_side_entry(
        &parsed.entries[0]
    ));

    let parsed =
        parse_style_property_entries_for_cssom_write("overflow", "overlay hidden", false, None)
            .expect("overflow shorthand should preserve overlay for CSSOM compat");
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries[0].name, "overflow-x");
    assert_eq!(parsed.entries[0].value, "overlay");
    assert_eq!(parsed.entries[1].name, "overflow-y");
    assert_eq!(parsed.entries[1].value, "hidden");
    assert!(style_entry_is_pdb_supplemental_side_entry(
        &parsed.entries[0]
    ));
    assert!(!style_entry_is_pdb_supplemental_side_entry(
        &parsed.entries[1]
    ));
}

#[test]
fn css_variable_specified_values_bypass_structured_property_expansion() {
    let margin = parse_style_property_entries_with_base("margin", "var(--prop)", true, None)
        .expect("valid var() margin shorthand should parse as specified value");
    assert_eq!(margin.entries.len(), 1);
    assert_eq!(margin.entries[0].name, "margin");
    assert_eq!(margin.entries[0].value, "var(--prop)");
    assert!(margin.entries[0].priority);
    assert_eq!(margin.affected_names, ["margin"]);

    assert!(parse_style_property_entries_with_base("width", "var(--x ())", false, None).is_none());
    assert!(
        parse_style_property_entries_with_base("expando", "var(--prop)", false, None).is_none()
    );
}

#[test]
fn custom_property_entries_preserve_empty_specified_value_semantics() {
    let empty = parse_style_property_entries_with_base("--var", "", false, None)
        .expect("empty custom property value should parse");
    assert_eq!(empty.entries[0].name, "--var");
    assert_eq!(empty.entries[0].value, "");

    let whitespace = parse_style_property_entries_with_base("--var", "  ", false, None)
        .expect("whitespace custom property value should parse");
    assert_eq!(whitespace.entries[0].value, "");

    let value = parse_style_property_entries_with_base("--var", " value  ", false, None)
        .expect("non-empty custom property value should parse");
    assert_eq!(value.entries[0].value, "value");

    assert!(
        parse_style_property_entries_for_cssom_write("--var", "a;b", false, None).is_none(),
        "CSSOM custom property values reject bare top-level semicolons"
    );
    let escaped_semicolon =
        parse_style_property_entries_for_cssom_write("--var", r#"a\;b"#, false, None)
            .expect("CSSOM custom property values accept escaped top-level semicolons");
    assert_eq!(escaped_semicolon.entries[0].value, r#"a\;b"#);
    assert!(
        parse_style_property_entries_for_cssom_write("--var", r#"Hello\; world!"#, false, None)
            .is_none(),
        "CSSOM custom property values reject bare priority delimiters"
    );
    let escaped_priority =
        parse_style_property_entries_for_cssom_write("--var", r#"Hello\; world\!"#, false, None)
            .expect("CSSOM custom property values accept escaped priority delimiters");
    assert_eq!(escaped_priority.entries[0].value, r#"Hello\; world\!"#);

    assert!(parse_style_property_entries_with_base("--", "value", false, None).is_none());
    assert!(parse_style_property_entries_with_base("--var name", "value", false, None).is_none());
}

#[test]
fn custom_property_entries_accept_ident_var_reference_names() {
    let parsed = parse_style_property_entries_with_base(
        "--var-with-ident",
        r#"var(ident("--myprop" calc(3 * sign(1em - 1px))), FAIL)"#,
        false,
        None,
    )
    .expect("custom property value should preserve ident() var reference names");
    assert_eq!(parsed.entries[0].name, "--var-with-ident");
    assert_eq!(
        parsed.entries[0].value,
        r#"var(ident("--myprop" calc(3 * sign(1em - 1px))), FAIL)"#
    );
}

#[test]
fn animation_timing_function_parser_preserves_linear_round_trip_percent_precision() {
    let parsed = parse_style_property_entries_with_base(
            "animation-timing-function",
            "linear(0 0%, 1.3 11.111111%, 1 22.222222%, 0.92 33.333333%, 1 44.444444%, 0.99 55.555556%, 1 66.666667%, 1.004 77.777778%, 0.998 88.888889%, 1 100%, 1 100%)",
            false,
            None,
        )
        .expect("linear easing should parse");
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].name, "animation-timing-function");
    assert_eq!(
        parsed.entries[0].value,
        "linear(0 0%, 1.3 11.111111%, 1 22.222222%, 0.92 33.333333%, 1 44.444444%, 0.99 55.555556%, 1 66.666667%, 1.004 77.777778%, 0.998 88.888889%, 1 100%, 1 100%)"
    );
}

#[test]
fn content_alt_counters_use_stylo_declaration_blocks() {
    for (input, expected) in [
        ("\"\" / counter(cnt)", "\"\" / counter(cnt)"),
        (
            "\"regular text\" / \"alt text 1\" counter(cnt) \"alt text 2\"",
            "\"regular text\" / \"alt text 1\" counter(cnt) \"alt text 2\"",
        ),
        (
            "\"regular text\" / counter(cnt) \"alt text\"",
            "\"regular text\" / counter(cnt) \"alt text\"",
        ),
        (
            "\"regular text\" / counters(chapter, \".\", DECIMAL)",
            "\"regular text\" / counters(chapter, \".\")",
        ),
        (
            "\"main / label\" / /* alt */ counter(cnt)",
            "\"main / label\" / counter(cnt)",
        ),
    ] {
        let parsed = parse_style_property_entries_for_cssom_write("content", input, true, None)
            .unwrap_or_else(|| panic!("content: {input} should parse"));
        assert_eq!(parsed.entries.len(), 1, "content: {input}");
        assert_eq!(parsed.entries[0].name, "content", "content: {input}");
        assert_eq!(parsed.entries[0].value, expected, "content: {input}");
        assert!(parsed.entries[0].priority, "content: {input}");
        assert_eq!(parsed.affected_names, vec!["content".to_owned()]);
        assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
        assert!(
            !style_entry_is_pdb_supplemental_side_entry(&parsed.entries[0]),
            "content: {input} should use the native Stylo declaration block"
        );
    }

    for invalid in [
        "none / counter(cnt)",
        "\"\" / counter()",
        "\"\" / url(alt.svg) counter(cnt)",
        "\"\" / open-quote counter(cnt)",
        "\"\" / counter(cnt) / \"extra\"",
        "\"\" / counter(cnt) }",
    ] {
        assert!(
            parse_style_property_entries_for_cssom_write("content", invalid, false, None).is_none(),
            "content: {invalid} should remain invalid"
        );
    }

    let ordinary = parse_style_property_entries_for_cssom_write(
        "content",
        "\"regular text\" / \"alt text\"",
        false,
        None,
    )
    .expect("ordinary string alt text should remain Stylo-backed");
    assert!(!style_entry_is_pdb_supplemental_side_entry(
        &ordinary.entries[0]
    ));
}
