use crate::{
    css_style::{
        box_shorthand_value_components, canonical_style_property_identifier,
        canonical_style_property_name,
    },
    detached_css_style::css_style_declaration_exposes_property_name,
    document_runtime::DomHandle,
};

use super::super::super::super::JsContextHost;
use super::values::computed_style_default_value;
use super::{StyleMode, style_entries};

pub(in crate::native_bridge::element::styles) fn is_style_intrinsic_name(name: &str) -> bool {
    matches!(
        name,
        "length"
            | "cssText"
            | "setProperty"
            | "getPropertyValue"
            | "removeProperty"
            | "getPropertyPriority"
            | "item"
            | "toString"
    )
}

pub(in crate::native_bridge::element::styles) fn known_style_property(property: &str) -> bool {
    css_style_declaration_exposes_property_name(property)
}

pub(in crate::native_bridge::element::styles) fn all_shorthand_applies_to(property: &str) -> bool {
    !property.starts_with("--")
        && !matches!(property, "all" | "direction" | "unicode-bidi")
        && known_style_property(property)
}

pub(in crate::native_bridge::element::styles) fn css_wide_keyword(value: &str) -> Option<String> {
    let lowered = value.trim().to_ascii_lowercase();
    matches!(
        lowered.as_str(),
        "initial" | "inherit" | "unset" | "revert" | "revert-layer" | "revert-rule"
    )
    .then_some(lowered)
}

pub(in crate::native_bridge::element::styles) fn supported_declared_property(
    property: &str,
) -> bool {
    moli_css_parse::is_cssom_custom_property_name(property) || known_style_property(property)
}

pub(in crate::native_bridge::element::styles) fn shorthand_longhands(
    property: &str,
) -> Option<&'static [&'static str]> {
    match property {
        "animation" => Some(animation_shorthand_longhands()),
        "animation-range" => Some(&["animation-range-start", "animation-range-end"]),
        "transition" => Some(transition_shorthand_longhands()),
        "text-decoration" => Some(text_decoration_shorthand_affected_longhands()),
        "text-emphasis" => Some(&["text-emphasis-style", "text-emphasis-color"]),
        "-webkit-text-stroke" => Some(&["-webkit-text-stroke-width", "-webkit-text-stroke-color"]),
        "flex" => Some(&["flex-grow", "flex-shrink", "flex-basis"]),
        "flex-flow" => Some(&["flex-direction", "flex-wrap"]),
        "inset" => Some(&["top", "right", "bottom", "left"]),
        "margin" => Some(&["margin-top", "margin-right", "margin-bottom", "margin-left"]),
        "margin-inline" => Some(&["margin-inline-start", "margin-inline-end"]),
        "margin-block" => Some(&["margin-block-start", "margin-block-end"]),
        "grid-column" => Some(&["grid-column-start", "grid-column-end"]),
        "list-style" => Some(&["list-style-position", "list-style-type", "list-style-image"]),
        "outline" => Some(&["outline-color", "outline-style", "outline-width"]),
        "border-color" => Some(&[
            "border-top-color",
            "border-right-color",
            "border-bottom-color",
            "border-left-color",
        ]),
        "border-style" => Some(&[
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
        ]),
        "border-width" => Some(&[
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ]),
        "border-radius" => Some(&[
            "border-top-left-radius",
            "border-top-right-radius",
            "border-bottom-right-radius",
            "border-bottom-left-radius",
        ]),
        "padding" => Some(&[
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ]),
        "font" => Some(font_shorthand_longhands()),
        "overscroll-behavior" => Some(&["overscroll-behavior-x", "overscroll-behavior-y"]),
        _ => None,
    }
}

pub(in crate::native_bridge::element::styles) fn font_shorthand_longhands()
-> &'static [&'static str] {
    &[
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
    ]
}

pub(in crate::native_bridge::element::styles) fn font_variant_longhands() -> &'static [&'static str]
{
    &[
        "font-variant-ligatures",
        "font-variant-caps",
        "font-variant-alternates",
        "font-variant-numeric",
        "font-variant-east-asian",
        "font-variant-position",
        "font-variant-emoji",
    ]
}

pub(in crate::native_bridge::element::styles) fn animation_shorthand_longhands()
-> &'static [&'static str] {
    &[
        "animation-duration",
        "animation-timing-function",
        "animation-delay",
        "animation-iteration-count",
        "animation-direction",
        "animation-fill-mode",
        "animation-play-state",
        "animation-name",
    ]
}

pub(in crate::native_bridge::element::styles) fn transition_shorthand_longhands()
-> &'static [&'static str] {
    &[
        "transition-property",
        "transition-duration",
        "transition-timing-function",
        "transition-delay",
        "transition-behavior",
    ]
}

pub(in crate::native_bridge::element::styles) fn text_decoration_shorthand_longhands()
-> &'static [&'static str] {
    &[
        "text-decoration-line",
        "text-decoration-thickness",
        "text-decoration-style",
        "text-decoration-color",
    ]
}

pub(in crate::native_bridge::element::styles) fn text_decoration_shorthand_affected_longhands()
-> &'static [&'static str] {
    &[
        "text-decoration-line",
        "text-decoration-thickness",
        "text-decoration-style",
        "text-decoration-color",
        "text-decoration-fill",
        "text-decoration-inset",
        "text-decoration-skip-ink",
        "text-decoration-skip-spaces",
        "text-decoration-stroke",
    ]
}

pub(in crate::native_bridge::element::styles) fn box_shorthand_components(
    value: &str,
) -> Option<[String; 4]> {
    let components = box_shorthand_value_components(value)?;
    match components.as_slice() {
        [single] => Some(std::array::from_fn(|_| single.clone())),
        [vertical, horizontal] => Some([
            vertical.clone(),
            horizontal.clone(),
            vertical.clone(),
            horizontal.clone(),
        ]),
        [top, horizontal, bottom] => Some([
            top.clone(),
            horizontal.clone(),
            bottom.clone(),
            horizontal.clone(),
        ]),
        [top, right, bottom, left] => {
            Some([top.clone(), right.clone(), bottom.clone(), left.clone()])
        }
        _ => None,
    }
}

fn inline_style_has_property(runtime: &JsContextHost, handle: DomHandle, property: &str) -> bool {
    let property = canonical_style_property_name(property);
    style_entries(runtime, handle)
        .iter()
        .any(|entry| entry.name == property)
}

fn style_property_exists(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
) -> bool {
    inline_style_has_property(runtime, handle, property)
        || known_style_property(property)
        || (mode == StyleMode::Computed
            && !computed_style_default_value(runtime, handle, property).is_empty())
}

fn canonical_property_name_for_named_lookup(raw_key: &str) -> Option<String> {
    if raw_key.is_empty() || is_style_intrinsic_name(raw_key) {
        return None;
    }
    if raw_key == "cssFloat" {
        return Some("float".to_owned());
    }
    if raw_key.starts_with('-') {
        return Some(canonical_style_property_name(raw_key));
    }
    for prefix in ["moz", "ms", "o"] {
        if let Some(suffix) = raw_key.strip_prefix(prefix)
            && suffix
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
        {
            return None;
        }
    }
    Some(canonical_style_property_identifier(raw_key))
}

pub(in crate::native_bridge::element::styles) fn resolve_known_style_property_name(
    raw_key: &str,
) -> Option<String> {
    let property = canonical_property_name_for_named_lookup(raw_key)?;
    known_style_property(&property).then_some(property)
}

pub(in crate::native_bridge::element::styles) fn resolve_style_property_name(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    raw_key: &str,
) -> Option<String> {
    let property = canonical_property_name_for_named_lookup(raw_key)?;
    style_property_exists(runtime, handle, mode, &property).then_some(property)
}

#[cfg(test)]
mod tests {
    use super::box_shorthand_components;

    #[test]
    fn box_shorthand_components_keep_function_whitespace_internal() {
        assert_eq!(
            box_shorthand_components("calc(10px + 5px) var(--gap, 1px 2px)").as_ref(),
            Some(&[
                "calc(10px + 5px)".to_owned(),
                "var(--gap, 1px 2px)".to_owned(),
                "calc(10px + 5px)".to_owned(),
                "var(--gap, 1px 2px)".to_owned(),
            ])
        );
    }
}
