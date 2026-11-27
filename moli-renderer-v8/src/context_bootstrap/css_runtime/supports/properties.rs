use super::*;
use crate::{
    css_style::{mask_compat_property_name, mask_compat_value_is_supported},
    detached_css_style::css_style_declaration_exposes_property_name,
    native_bridge::element::parse_cssom_style_property_entries_for_write,
};

pub(in crate::context_bootstrap::css_runtime::supports) fn css_supports_property_value(
    property: &str,
    value: &str,
) -> bool {
    let property_trimmed = property.trim();
    let value_trimmed = value.trim();
    if property_trimmed.is_empty()
        || value_trimmed.is_empty()
        || property_trimmed != property
        || property_trimmed.contains(char::is_whitespace)
        || property_trimmed.contains(':')
        || property_trimmed.contains('\\')
        || value_trimmed.contains('\n')
        || moli_css_parse::split_important_priority(value_trimmed).1
    {
        return false;
    }

    let property_lower = property_trimmed.to_ascii_lowercase();
    let canonical_property = moli_css_parse::canonical_style_property_name(property_trimmed);
    let value_lower = value_trimmed.to_ascii_lowercase();

    if property_lower.starts_with("--")
        && !moli_css_parse::is_cssom_custom_property_name(property_trimmed)
    {
        return false;
    }
    if !property_lower.starts_with("--")
        && !css_style_declaration_exposes_property_name(&canonical_property)
    {
        return false;
    }

    if !moli_css_parse::css_declaration_value_has_valid_env_functions(value_trimmed) {
        return false;
    }

    if super::stylo_supports_property_value(&canonical_property, value_trimmed) {
        return true;
    }

    if values::is_css_wide_keyword(&value_lower) && supports_css_wide_keywords(&canonical_property)
    {
        return true;
    }

    if legacy_css_supports_property_value(&property_lower, &value_lower) {
        return true;
    }

    if legacy_css_supports_property_name(&property_lower) {
        return false;
    }

    cssom_write_compat_supports_property_value(&canonical_property, value_trimmed)
}

fn supports_css_wide_keywords(property: &str) -> bool {
    property.starts_with("--") || css_style_declaration_exposes_property_name(property)
}

fn cssom_write_compat_supports_property_value(property: &str, value: &str) -> bool {
    css_style_declaration_exposes_property_name(property)
        && parse_cssom_style_property_entries_for_write(property, value, false, None).is_some()
}

fn legacy_css_supports_property_value(property: &str, value: &str) -> bool {
    match property {
        "background-image" => values::is_legacy_webkit_background_image_like(value),
        _ => mask_compat_value_is_supported(property, value),
    }
}

fn legacy_css_supports_property_name(property: &str) -> bool {
    mask_compat_property_name(property)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_supports_matches_chromium_for_former_gecko_property_gates() {
        for property in crate::chromium_property_surface::FORMER_GECKO_GATED_SUPPORTED_PROPERTIES {
            assert!(
                css_supports_property_value(property, "initial"),
                "Chromium-supported property should accept initial: {property}"
            );
        }
        for property in crate::chromium_property_surface::GECKO_ONLY_UNSUPPORTED_PROPERTIES {
            assert!(
                !css_supports_property_value(property, "initial"),
                "Gecko-only property must stay unsupported: {property}"
            );
        }

        for axis in ["x", "y"] {
            assert!(!css_supports_property_value(
                &format!("mask-position-{axis}"),
                "initial"
            ));
            assert!(css_supports_property_value(
                &format!("-webkit-mask-position-{axis}"),
                "initial"
            ));
        }
    }

    #[test]
    fn css_supports_property_value_uses_pdb_value_fragment_boundaries() {
        assert!(css_supports_property_value("content", r#""a;b""#));
        assert!(!css_supports_property_value("display", "block; color: red"));
        assert!(!css_supports_property_value("display", "block !important"));
        assert!(css_supports_property_value("--feature", "calc(1px + 2px)"));
        assert!(css_supports_property_value("link-parameters", "param(--a"));
        assert!(!css_supports_property_value("--", "initial"));
    }

    #[test]
    fn css_supports_math_properties_use_stylo_parser() {
        assert!(css_supports_property_value(
            "width",
            "calc(10px + 1vmin + 10%)"
        ));
        assert!(css_supports_property_value(
            "margin-top",
            "clamp(1px,2px,3px)"
        ));
        assert!(css_supports_property_value("tab-size", "calc(2 * 3)"));
        assert!(!css_supports_property_value("width", "calc(7px * up)"));
        assert!(!css_supports_property_value(
            "transform",
            "rotate(calc((0.25turn error)))"
        ));
        assert!(!css_supports_property_value(
            "width",
            "round(nearest, 1px, 1px, 1px)"
        ));
    }

    #[test]
    fn css_supports_color_properties_use_stylo_parser() {
        for property in ["color", "background-color"] {
            assert!(css_supports_property_value(property, "rgb(10 0 0)"));
            assert!(!css_supports_property_value(
                property,
                "rgb(clamp(10, none, 20) 0 0)"
            ));
        }
    }

    #[test]
    fn css_supports_image_set_uses_stylo_parser() {
        assert!(css_supports_property_value(
            "background-image",
            r#"image-set(url("") calc(1x * NaN))"#
        ));
    }

    #[test]
    fn css_supports_background_image_basic_values_do_not_need_legacy_helper() {
        assert!(css_supports_property_value("background-image", "none"));
        assert!(css_supports_property_value(
            "background-image",
            r#"url("/plain.png")"#
        ));
        assert!(!values::is_legacy_webkit_background_image_like("none"));
        assert!(!values::is_legacy_webkit_background_image_like(
            r#"url("/plain.png")"#
        ));
    }

    #[test]
    fn css_supports_mask_compat_values_use_narrow_boundary() {
        assert!(css_supports_property_value("-webkit-appearance", "auto"));
        assert!(!css_supports_property_value("-webkit-appearance", "banana"));
        assert!(!css_supports_property_value("-moz-user-select", "none"));
        assert!(!css_supports_property_value("-moz-user-select", "inherit"));
        assert!(css_supports_property_value("mask", "none"));
        assert!(!css_supports_property_value("mask", "banana"));
        assert!(css_supports_property_value("-webkit-mask-repeat", "repeat"));
        assert!(!css_supports_property_value(
            "-webkit-mask-repeat",
            "repeat invalid"
        ));
    }

    #[test]
    fn css_supports_uses_stylo_for_css_wide_keywords() {
        assert!(css_supports_property_value("display", "initial"));
        assert!(css_supports_property_value("color", "revert-rule"));
        assert!(css_supports_property_value("z-index", "revert-rule"));
        assert!(css_supports_property_value("align-content", "inherit"));
        assert!(css_supports_property_value("border-spacing", "inherit"));
        assert!(css_supports_property_value("list-style", "inherit"));
        assert!(css_supports_property_value("outline", "inherit"));
        assert!(css_supports_property_value(
            "text-decoration-color",
            "inherit"
        ));
        assert!(css_supports_property_value(
            "-webkit-align-content",
            "inherit"
        ));
        assert!(css_supports_property_value(
            "-webkit-text-fill-color",
            "inherit"
        ));
        assert!(!css_supports_property_value("-moz-user-select", "inherit"));
        assert!(css_supports_property_value(
            "border-bottom-color",
            "inherit"
        ));
        assert!(!css_supports_property_value("size", "initial"));
        assert!(!css_supports_property_value("page-orientation", "initial"));
    }

    #[test]
    fn css_supports_accepts_outline_color_invert_via_cssom_write_path() {
        assert!(css_supports_property_value("outline-color", "invert"));
        assert!(css_supports_property_value("outline-color", "InVeRt"));
        assert!(!css_supports_property_value("outline-color", "invert red"));
    }

    #[test]
    fn css_supports_accepts_overflow_overlay_via_cssom_write_path() {
        assert!(css_supports_property_value("overflow-x", "overlay"));
        assert!(css_supports_property_value("overflow", "overlay hidden"));
        assert!(!css_supports_property_value("overflow", "overlay invalid"));
    }

    #[test]
    fn css_supports_text_decoration_compat_uses_cssom_write_path() {
        assert!(css_supports_property_value(
            "text-decoration-line",
            "spelling-error"
        ));
        assert!(css_supports_property_value(
            "text-decoration-line",
            "Grammar-Error"
        ));
        assert!(!css_supports_property_value(
            "text-decoration-line",
            "spelling-error underline"
        ));
        assert!(!css_supports_property_value(
            "text-decoration-style",
            "blink"
        ));
    }

    #[test]
    fn css_supports_rejects_css_wide_keywords_mixed_with_ordinary_values() {
        assert!(!css_supports_property_value(
            "border-spacing",
            "5px inherit"
        ));
        assert!(!css_supports_property_value("margin", "inherit 5px"));
        assert!(!css_supports_property_value("overflow", "inherit scroll"));
    }

    #[test]
    fn css_supports_container_query_properties() {
        assert!(css_supports_property_value("container-type", "normal"));
        assert!(css_supports_property_value("container-type", "size"));
        assert!(css_supports_property_value("container-type", "inline-size"));
        assert!(!css_supports_property_value("container-type", "banana"));

        assert!(css_supports_property_value("container-name", "none"));
        assert!(css_supports_property_value("container-name", "card"));
        assert!(css_supports_property_value("container-name", "--"));
        assert!(css_supports_property_value("container-name", "--foo"));
        assert!(css_supports_property_value("container-name", "-foo"));
        assert!(css_supports_property_value(
            "container-name",
            "card sidebar"
        ));
        assert!(!css_supports_property_value("container-name", ""));
        assert!(!css_supports_property_value("container-name", "-"));
        assert!(!css_supports_property_value("container-name", "-1"));
        assert!(!css_supports_property_value("container-name", "1card"));
        assert!(!css_supports_property_value("container-name", "none card"));
        assert!(!css_supports_property_value(
            "container-name",
            "initial card"
        ));

        assert!(css_supports_property_value("container", "normal"));
        assert!(css_supports_property_value("container", "card"));
        assert!(css_supports_property_value(
            "container",
            "card / inline-size"
        ));
        assert!(css_supports_property_value(
            "container",
            "card sidebar / size"
        ));
        assert!(css_supports_property_value("container", "-- / inline-size"));
        assert!(!css_supports_property_value("container", "/ inline-size"));
        assert!(!css_supports_property_value("container", "- / inline-size"));
        assert!(!css_supports_property_value(
            "container",
            "-1 / inline-size"
        ));
        assert!(!css_supports_property_value("container", "card / banana"));
    }

    #[test]
    fn css_supports_accepts_transform_none() {
        assert!(css_supports_property_value("transform", "none"));
    }

    #[test]
    fn css_supports_accepts_legacy_webkit_gradient_background_images() {
        assert!(css_supports_property_value(
            "background-image",
            "-webkit-radial-gradient(1px 2px, 3% 4%, red, blue)"
        ));
        assert!(!css_supports_property_value(
            "background-image",
            "-webkit-radial-gradient(1px 2px, 3% 4% red, blue)"
        ));
        assert!(!css_supports_property_value(
            "background-image",
            "-webkit-radial-gradient(1px 2px, -3% 4%, red, blue)"
        ));
        assert!(css_supports_property_value(
            "background-image",
            "-webkit-gradient(linear, left top, left bottom, color-stop(calc(0.5 + 0.001 * sign(1em - 1px)), blue))"
        ));
    }

    #[test]
    fn css_supports_uses_cssom_parser_for_exposed_animation_properties() {
        assert!(css_supports_property_value("animation", "1s"));
        assert!(css_supports_property_value(
            "animation",
            "anim paused both reverse 4 1s -3s cubic-bezier(0, -2, 1, 3)"
        ));
        assert!(css_supports_property_value("animation-timeline", "auto"));
        assert!(css_supports_property_value(
            "animation-range-start",
            "normal"
        ));
        assert!(css_supports_property_value("animation-range-end", "normal"));
        assert!(css_supports_property_value(
            "animation-duration",
            "calc(10s + (sign(2cqw - 10px) * 5s))"
        ));
        assert!(css_supports_property_value(
            "animation-iteration-count",
            "calc(10 + (sign(2cqw - 10px) * 5))"
        ));
        assert!(!css_supports_property_value("animation-duration", "-3s"));
        assert!(css_supports_property_value(
            "transition-delay",
            "calc(10s + (sign(2cqw - 10px) * 5s))"
        ));
        assert!(css_supports_property_value(
            "transition-duration",
            "calc(10s + (sign(2cqw - 10px) * 5s))"
        ));
        assert!(!css_supports_property_value("transition-duration", "-3s"));
        assert!(css_supports_property_value(
            "transition-timing-function",
            "steps(sibling-index(), jump-none)"
        ));
        assert!(css_supports_property_value(
            "transition-timing-function",
            "steps(calc(2 * sign(1em - 1000px)), start)"
        ));
        assert!(css_supports_property_value(
            "animation-timing-function",
            "steps(calc(1), jump-none)"
        ));
        assert!(!css_supports_property_value(
            "animation-timing-function",
            "steps(calc(0/0), jump-none)"
        ));
    }

    #[test]
    fn css_supports_uses_stylo_for_exposed_paged_media_properties() {
        assert!(css_supports_property_value("orphans", "2"));
        assert!(css_supports_property_value("widows", "2"));
        assert!(!css_supports_property_value("orphans", "0"));
        assert!(css_supports_property_value("page-break-after", "auto"));
        assert!(css_supports_property_value("page-break-before", "left"));
        assert!(css_supports_property_value("page-break-inside", "avoid"));
        assert!(!css_supports_property_value("page-break-inside", "left"));
    }
}
