use super::{
    Attribute, CustomElementState, Element, html_element_interface_name,
    mathml_element_interface_name, svg_element_interface_name,
};
use crate::native::NativeNodeId;

#[test]
fn ordinary_elements_keep_control_state_unmaterialized_until_needed() {
    let mut div = Element::new_html("div");
    assert!(!div.rare_data.is_materialized());

    // Reading default state must not allocate an element-owned rare payload.
    assert_eq!(div.scroll_top(), 0.0);
    assert!(!div.set_scroll_top(0.0));
    assert!(!div.set_scroll_left(0.0));
    assert!(!div.set_cryptographic_nonce(None));
    assert!(!div.popover_open());
    assert!(div.custom_states().is_empty());
    assert!(!div.rare_data.is_materialized());

    // Unrelated parser/DOM attributes must stay on the common element path.
    assert!(div.set_attribute("class".to_owned(), String::new(), None, "card".to_owned(),));
    assert!(!div.rare_data.is_materialized());

    assert!(div.set_scroll_top(12.0));
    assert_eq!(div.scroll_top(), 12.0);
    assert!(div.rare_data.is_materialized());
}

#[test]
fn stateful_element_kinds_materialize_control_state_during_construction() {
    for local_name in ["input", "textarea", "option", "audio", "video", "script"] {
        let element = Element::new_html(local_name);
        assert!(
            element.rare_data.is_materialized(),
            "{local_name} requires initial control state"
        );
    }
}

#[test]
fn nonce_attribute_materializes_rare_state_without_affecting_unrelated_attributes() {
    let mut div = Element::new_html("div");
    assert!(div.set_attribute("nonce".to_owned(), String::new(), None, "token".to_owned(),));
    assert_eq!(div.cryptographic_nonce(), Some("token"));
    assert!(div.rare_data.is_materialized());

    assert!(div.remove_attribute("nonce"));
    assert_eq!(div.cryptographic_nonce(), None);
}

#[test]
fn cloned_elements_do_not_share_materialized_rare_state() {
    let mut original = Element::new_html("div");
    assert!(original.set_scroll_top(12.0));
    assert!(original.set_custom_element_is_name(Some("fancy-div".to_owned())));

    let mut cloned = original.clone();
    assert!(cloned.set_scroll_top(24.0));
    assert!(cloned.set_custom_element_is_name(Some("other-div".to_owned())));

    assert_eq!(original.scroll_top(), 12.0);
    assert_eq!(cloned.scroll_top(), 24.0);
    assert_eq!(original.custom_element_is_name(), Some("fancy-div"));
    assert_eq!(cloned.custom_element_is_name(), Some("other-div"));
}

#[test]
fn custom_element_is_name_uses_and_releases_rare_data() {
    let mut button = Element::new_html("button");
    assert!(!button.rare_data.is_materialized());

    assert!(button.set_custom_element_is_name(Some("fancy-button".to_owned())));
    assert_eq!(button.custom_element_is_name(), Some("fancy-button"));
    assert!(button.rare_data.is_materialized());

    assert!(button.set_custom_element_is_name(None));
    assert_eq!(button.custom_element_is_name(), None);
    assert!(!button.rare_data.is_materialized());
}

#[test]
fn parser_associated_form_owner_uses_and_releases_rare_data() {
    let mut button = Element::new_html("button");
    let owner = NativeNodeId::new(42);
    assert!(!button.rare_data.is_materialized());

    assert!(button.set_parser_associated_form_owner(Some(owner)));
    assert_eq!(button.parser_associated_form_owner(), Some(owner));
    assert!(button.rare_data.is_materialized());

    assert!(button.set_parser_associated_form_owner(None));
    assert_eq!(button.parser_associated_form_owner(), None);
    assert!(!button.rare_data.is_materialized());
}

#[test]
fn custom_element_state_uses_and_releases_rare_data() {
    let mut div = Element::new_html("div");
    assert_eq!(div.custom_element_state(), CustomElementState::Uncustomized);
    assert!(!div.rare_data.is_materialized());

    assert!(div.set_custom_element_state(CustomElementState::Undefined));
    assert_eq!(div.custom_element_state(), CustomElementState::Undefined);
    assert!(div.rare_data.is_materialized());

    assert!(div.set_custom_element_state(CustomElementState::Custom));
    assert_eq!(div.custom_element_state(), CustomElementState::Custom);

    assert!(div.set_custom_element_state(CustomElementState::Uncustomized));
    assert!(!div.rare_data.is_materialized());
}

#[test]
fn custom_element_candidates_materialize_their_initial_state() {
    let autonomous = Element::new_html("fancy-card");
    assert_eq!(
        autonomous.custom_element_state(),
        CustomElementState::Undefined
    );
    assert!(autonomous.rare_data.is_materialized());

    let customized_builtin = Element::new(
        "button".to_owned(),
        "http://www.w3.org/1999/xhtml".to_owned(),
        None,
        vec![Attribute::new(
            "is".to_owned(),
            String::new(),
            None,
            "fancy-button".to_owned(),
        )],
    );
    assert_eq!(
        customized_builtin.custom_element_state(),
        CustomElementState::Undefined
    );
    assert!(customized_builtin.rare_data.is_materialized());
}

#[test]
fn template_contents_uses_and_releases_rare_data() {
    let mut template = Element::new_html("template");
    let contents = NativeNodeId::new(7);
    assert_eq!(template.template_contents(), None);
    assert!(!template.rare_data.is_materialized());

    template.set_template_contents(Some(contents));
    assert_eq!(template.template_contents(), Some(contents));
    assert!(template.rare_data.is_materialized());

    template.set_template_contents(None);
    assert_eq!(template.template_contents(), None);
    assert!(!template.rare_data.is_materialized());
}

#[test]
fn element_reference_state_is_owned_by_the_content_attribute() {
    let mut element = Element::new_html("div");
    assert!(element.set_attribute(
        "aria-controls".to_owned(),
        String::new(),
        None,
        String::new(),
    ));
    element.set_explicit_element_references("aria-controls", vec![NativeNodeId::new(7)]);

    assert!(!element.set_attribute(
        "aria-controls".to_owned(),
        String::new(),
        None,
        String::new(),
    ));
    assert_eq!(element.explicit_element_references("aria-controls"), None);

    element.set_explicit_element_references("aria-controls", vec![NativeNodeId::new(8)]);
    assert!(element.set_attribute_ns(
        "aria-controls".to_owned(),
        "urn:example".to_owned(),
        Some("x".to_owned()),
        "foreign".to_owned(),
    ));
    assert_eq!(
        element.explicit_element_references("aria-controls"),
        Some([NativeNodeId::new(8)].as_slice())
    );

    assert!(element.remove_attribute("aria-controls"));
    assert_eq!(element.explicit_element_references("aria-controls"), None);

    assert!(element.set_attribute(
        "popovertarget".to_owned(),
        String::new(),
        None,
        String::new(),
    ));
    element.set_explicit_element_references("popovertarget", vec![NativeNodeId::new(9)]);
    assert!(!element.set_attribute(
        "popovertarget".to_owned(),
        String::new(),
        None,
        String::new(),
    ));
    assert_eq!(element.explicit_element_references("popovertarget"), None);
}

#[test]
fn html_element_interface_name_covers_replay_tags() {
    assert_eq!(html_element_interface_name("meta"), "HTMLMetaElement");
    assert_eq!(html_element_interface_name("span"), "HTMLSpanElement");
    assert_eq!(
        html_element_interface_name("tbody"),
        "HTMLTableSectionElement"
    );
    assert_eq!(html_element_interface_name("tr"), "HTMLTableRowElement");
    assert_eq!(html_element_interface_name("slot"), "HTMLSlotElement");
    assert_eq!(html_element_interface_name("applet"), "HTMLUnknownElement");
    assert_eq!(
        html_element_interface_name("menuitem"),
        "HTMLUnknownElement"
    );
    assert_eq!(html_element_interface_name("section"), "HTMLElement");
    assert_eq!(html_element_interface_name("basefont"), "HTMLElement");
    assert_eq!(html_element_interface_name("wbr"), "HTMLElement");
    assert_eq!(html_element_interface_name("unknown"), "HTMLUnknownElement");
    assert_eq!(html_element_interface_name("hi"), "HTMLUnknownElement");
    assert_eq!(html_element_interface_name("x-foo"), "HTMLElement");
}

#[test]
fn svg_element_interface_name_specializes_standard_elements() {
    assert_eq!(svg_element_interface_name("a"), "SVGAElement");
    assert_eq!(svg_element_interface_name("circle"), "SVGCircleElement");
    assert_eq!(svg_element_interface_name("clipPath"), "SVGClipPathElement");
    assert_eq!(svg_element_interface_name("defs"), "SVGDefsElement");
    assert_eq!(svg_element_interface_name("desc"), "SVGDescElement");
    assert_eq!(svg_element_interface_name("ellipse"), "SVGEllipseElement");
    assert_eq!(svg_element_interface_name("feBlend"), "SVGFEBlendElement");
    assert_eq!(
        svg_element_interface_name("feColorMatrix"),
        "SVGFEColorMatrixElement"
    );
    assert_eq!(
        svg_element_interface_name("feComposite"),
        "SVGFECompositeElement"
    );
    assert_eq!(
        svg_element_interface_name("feConvolveMatrix"),
        "SVGFEConvolveMatrixElement"
    );
    assert_eq!(
        svg_element_interface_name("feDiffuseLighting"),
        "SVGFEDiffuseLightingElement"
    );
    assert_eq!(
        svg_element_interface_name("feDisplacementMap"),
        "SVGFEDisplacementMapElement"
    );
    assert_eq!(
        svg_element_interface_name("feDistantLight"),
        "SVGFEDistantLightElement"
    );
    assert_eq!(
        svg_element_interface_name("feDropShadow"),
        "SVGFEDropShadowElement"
    );
    assert_eq!(svg_element_interface_name("feFuncA"), "SVGFEFuncAElement");
    assert_eq!(svg_element_interface_name("feFuncB"), "SVGFEFuncBElement");
    assert_eq!(svg_element_interface_name("feFuncG"), "SVGFEFuncGElement");
    assert_eq!(svg_element_interface_name("feFuncR"), "SVGFEFuncRElement");
    assert_eq!(
        svg_element_interface_name("feGaussianBlur"),
        "SVGFEGaussianBlurElement"
    );
    assert_eq!(
        svg_element_interface_name("feMorphology"),
        "SVGFEMorphologyElement"
    );
    assert_eq!(svg_element_interface_name("feOffset"), "SVGFEOffsetElement");
    assert_eq!(
        svg_element_interface_name("fePointLight"),
        "SVGFEPointLightElement"
    );
    assert_eq!(
        svg_element_interface_name("feSpecularLighting"),
        "SVGFESpecularLightingElement"
    );
    assert_eq!(
        svg_element_interface_name("feSpotLight"),
        "SVGFESpotLightElement"
    );
    assert_eq!(
        svg_element_interface_name("feTurbulence"),
        "SVGFETurbulenceElement"
    );
    assert_eq!(svg_element_interface_name("filter"), "SVGFilterElement");
    assert_eq!(
        svg_element_interface_name("foreignObject"),
        "SVGForeignObjectElement"
    );
    assert_eq!(svg_element_interface_name("g"), "SVGGElement");
    assert_eq!(svg_element_interface_name("image"), "SVGImageElement");
    assert_eq!(svg_element_interface_name("line"), "SVGLineElement");
    assert_eq!(
        svg_element_interface_name("linearGradient"),
        "SVGLinearGradientElement"
    );
    assert_eq!(svg_element_interface_name("metadata"), "SVGMetadataElement");
    assert_eq!(svg_element_interface_name("marker"), "SVGMarkerElement");
    assert_eq!(svg_element_interface_name("mask"), "SVGMaskElement");
    assert_eq!(svg_element_interface_name("path"), "SVGPathElement");
    assert_eq!(svg_element_interface_name("pattern"), "SVGPatternElement");
    assert_eq!(svg_element_interface_name("polygon"), "SVGPolygonElement");
    assert_eq!(svg_element_interface_name("polyline"), "SVGPolylineElement");
    assert_eq!(
        svg_element_interface_name("radialGradient"),
        "SVGRadialGradientElement"
    );
    assert_eq!(svg_element_interface_name("rect"), "SVGRectElement");
    assert_eq!(svg_element_interface_name("script"), "SVGScriptElement");
    assert_eq!(svg_element_interface_name("stop"), "SVGStopElement");
    assert_eq!(svg_element_interface_name("svg"), "SVGSVGElement");
    assert_eq!(svg_element_interface_name("symbol"), "SVGSymbolElement");
    assert_eq!(svg_element_interface_name("text"), "SVGTextElement");
    assert_eq!(svg_element_interface_name("textPath"), "SVGTextPathElement");
    assert_eq!(svg_element_interface_name("title"), "SVGTitleElement");
    assert_eq!(svg_element_interface_name("tspan"), "SVGTSpanElement");
    assert_eq!(svg_element_interface_name("use"), "SVGUseElement");
    assert_eq!(svg_element_interface_name("view"), "SVGViewElement");
    assert_eq!(svg_element_interface_name("style"), "SVGStyleElement");
    assert_eq!(svg_element_interface_name("custom"), "SVGElement");
}

#[test]
fn svg_text_content_role_covers_text_tspan_and_text_path() {
    for local_name in ["text", "tspan", "textPath"] {
        let element = Element::new(
            local_name.to_owned(),
            "http://www.w3.org/2000/svg".to_owned(),
            None,
            Vec::new(),
        );
        assert!(element.is_svg_text_content_element(), "{local_name}");
    }

    for (namespace, local_name) in [
        ("http://www.w3.org/2000/svg", "textpath"),
        ("http://www.w3.org/1999/xhtml", "text"),
    ] {
        let element = Element::new(
            local_name.to_owned(),
            namespace.to_owned(),
            None,
            Vec::new(),
        );
        assert!(
            !element.is_svg_text_content_element(),
            "{namespace} {local_name}"
        );
    }
}

#[test]
fn mathml_element_interface_name_uses_mathml_element() {
    assert_eq!(mathml_element_interface_name("math"), "MathMLElement");
    assert_eq!(mathml_element_interface_name("mrow"), "MathMLElement");
}

#[test]
fn tag_name_matching_uses_qualified_name_for_prefixed_elements() {
    let html_prefixed = Element::new(
        "aÇ".to_owned(),
        "http://www.w3.org/1999/xhtml".to_owned(),
        Some("test".to_owned()),
        Vec::new(),
    );

    assert!(html_prefixed.matches_tag_name("test:aÇ"));
    assert!(html_prefixed.matches_tag_name("TEST:AÇ"));
    assert!(!html_prefixed.matches_tag_name("test:aç"));
    assert!(!html_prefixed.matches_tag_name("aÇ"));

    let foreign_prefixed = Element::new(
        "st".to_owned(),
        "urn:moli:test".to_owned(),
        Some("te".to_owned()),
        Vec::new(),
    );

    assert!(foreign_prefixed.matches_tag_name("te:st"));
    assert!(!foreign_prefixed.matches_tag_name("st"));
    assert!(!foreign_prefixed.matches_tag_name("TE:ST"));
}

#[test]
fn set_attribute_preserves_existing_prefix_on_update() {
    let mut element = Element::new(
        "g".to_owned(),
        "http://www.w3.org/2000/svg".to_owned(),
        Some("svg".to_owned()),
        vec![Attribute::new(
            "kind".to_owned(),
            "urn:moli:test".to_owned(),
            Some("lm".to_owned()),
            "before".to_owned(),
        )],
    );

    assert!(element.set_attribute(
        "lm:kind".to_owned(),
        "urn:moli:test".to_owned(),
        Some("other".to_owned()),
        "after".to_owned(),
    ));

    let attribute = &element.attributes()[0];
    assert_eq!(attribute.prefix(), Some("lm"));
    assert_eq!(attribute.name(), "lm:kind");
    assert_eq!(attribute.value(), "after");
}

#[test]
fn input_type_change_sanitizes_without_dirtying_default_value() {
    let mut input = Element::new_html("input");
    assert_eq!(input.input_value(), "");
    assert!(!input.input_value_dirty());

    assert!(input.set_attribute("type".to_owned(), String::new(), None, "color".to_owned()));
    assert_eq!(input.input_value(), "#000000");
    assert!(!input.input_value_dirty());

    assert!(input.set_attribute("type".to_owned(), String::new(), None, "text".to_owned()));
    assert_eq!(input.input_value(), "");
    assert!(!input.input_value_dirty());

    assert!(input.set_attribute("type".to_owned(), String::new(), None, "color".to_owned()));
    assert!(input.set_input_value("#ffffff"));
    assert!(input.input_value_dirty());
    assert!(input.set_attribute("type".to_owned(), String::new(), None, "text".to_owned()));
    assert_eq!(input.input_value(), "#ffffff");
    assert!(input.input_value_dirty());
}

#[test]
fn range_input_value_uses_live_min_max_and_step_attributes() {
    let attribute = |name: &str, value: &str| {
        Attribute::new(name.to_owned(), String::new(), None, value.to_owned())
    };
    let mut input = Element::new(
        "input".to_owned(),
        "http://www.w3.org/1999/xhtml".to_owned(),
        None,
        vec![
            attribute("type", "range"),
            attribute("min", "0"),
            attribute("max", "100"),
            attribute("step", "20"),
            attribute("value", "40"),
        ],
    );

    assert_eq!(input.input_value(), "40");
    assert!(input.set_input_value(""));
    assert_eq!(input.input_value(), "60");
    assert!(input.input_value_dirty());

    assert!(input.set_input_value("80"));
    assert!(input.set_attribute("max".to_owned(), String::new(), None, "50".to_owned()));
    assert_eq!(input.input_value(), "40");
    assert!(input.input_value_dirty());

    let defaulted = Element::new(
        "input".to_owned(),
        "http://www.w3.org/1999/xhtml".to_owned(),
        None,
        vec![
            attribute("type", "range"),
            attribute("min", "2"),
            attribute("max", "6"),
        ],
    );
    assert_eq!(defaulted.input_value(), "4");
    assert!(!defaulted.input_value_dirty());
}

#[test]
fn email_multiple_attribute_resanitizes_dirty_value() {
    let mut input = Element::new_html("input");
    assert!(input.set_attribute("type".to_owned(), String::new(), None, "email".to_owned()));
    assert!(input.set_input_value("  first@example.com  , second@example.test  "));
    assert_eq!(
        input.input_value(),
        "first@example.com  , second@example.test"
    );
    assert!(input.input_value_dirty());

    assert!(input.set_attribute("multiple".to_owned(), String::new(), None, String::new()));
    assert_eq!(input.input_value(), "first@example.com,second@example.test");
    assert!(input.input_value_dirty());
}

#[test]
fn heading_reflections_parse_range_and_modal_state() {
    let mut heading = Element::new_html("h1");
    assert_eq!(heading.heading_offset(), 0);
    assert!(!heading.heading_reset());

    for (raw, expected) in [
        ("3", 3),
        (" +7tail", 7),
        ("20", 8),
        ("429496729600", 8),
        ("-0", 0),
        ("-3", 0),
        ("invalid", 0),
        ("\u{000b}7", 0),
    ] {
        assert!(heading.set_attribute(
            "headingoffset".to_owned(),
            String::new(),
            None,
            raw.to_owned(),
        ));
        assert_eq!(heading.heading_offset(), expected, "{raw:?}");
    }

    assert!(heading.set_attribute(
        "headingreset".to_owned(),
        String::new(),
        None,
        String::new(),
    ));
    assert!(heading.heading_reset());

    let mut dialog = Element::new_html("dialog");
    assert!(!dialog.heading_reset());
    assert!(dialog.set_dialog_modal(true));
    assert!(dialog.heading_reset());
    assert!(dialog.set_dialog_modal(false));
    assert!(!dialog.heading_reset());
}

#[test]
fn script_element_state_distinguishes_dynamic_and_parser_created_scripts() {
    let dynamic = Element::new_html("script");
    assert!(dynamic.script_async());
    assert!(!dynamic.script_already_started());
    assert!(!dynamic.script_parser_inserted_for_prepare());

    let parser_created = Element::new_parser_created(
        "script".to_owned(),
        "http://www.w3.org/1999/xhtml".to_owned(),
        None,
        Vec::new(),
    );
    assert!(!parser_created.script_async());
    assert!(!parser_created.script_already_started());
    assert!(parser_created.script_parser_inserted_for_prepare());

    let dynamic_svg = Element::new(
        "script".to_owned(),
        "http://www.w3.org/2000/svg".to_owned(),
        None,
        vec![Attribute::new(
            "href".to_owned(),
            String::new(),
            None,
            "/dynamic.js".to_owned(),
        )],
    );
    assert!(dynamic_svg.is_script_element());
    assert!(dynamic_svg.script_async());
    assert_eq!(dynamic_svg.script_source_attribute(), Some("/dynamic.js"));

    let xlink_svg = Element::new(
        "script".to_owned(),
        "http://www.w3.org/2000/svg".to_owned(),
        None,
        vec![Attribute::new(
            "href".to_owned(),
            "http://www.w3.org/1999/xlink".to_owned(),
            Some("xlink".to_owned()),
            "/legacy.js".to_owned(),
        )],
    );
    assert_eq!(xlink_svg.script_source_attribute(), Some("/legacy.js"));

    let parser_created_svg = Element::new_parser_created(
        "script".to_owned(),
        "http://www.w3.org/2000/svg".to_owned(),
        None,
        Vec::new(),
    );
    assert!(!parser_created_svg.script_async());
    assert!(parser_created_svg.script_parser_inserted_for_prepare());
}

#[test]
fn parser_created_link_processing_state_is_consumed_at_children_finish() {
    let dynamic = Element::new_html("link");
    assert!(!dynamic.link_created_by_parser());

    let mut parser_created = Element::new_parser_created(
        "link".to_owned(),
        "http://www.w3.org/1999/xhtml".to_owned(),
        None,
        Vec::new(),
    );
    assert!(parser_created.link_created_by_parser());
    assert!(parser_created.finish_parsing_link_children());
    assert!(!parser_created.link_created_by_parser());
    assert!(!parser_created.finish_parsing_link_children());
}
