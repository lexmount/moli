// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Presentational hints belong at the Stylo adapter boundary. Keeping legacy
// HTML attributes and SVG presentation attributes here lets the normal
// cascade, inheritance, and relative-length resolver own the result instead
// of teaching layout about authored attribute strings.

use cssparser::serialize_string;
use selectors::{Element as SelectorsElement, sink::Push};
use style::{
    applicable_declarations::ApplicableDeclarationBlock,
    context::QuirksMode,
    properties::{
        Importance, PropertyDeclaration, PropertyDeclarationBlock, PropertyId,
        SourcePropertyDeclaration, parse_one_declaration_into,
    },
    rule_tree::{CascadeLevel, CascadeOrigin},
    servo_arc::Arc,
    stylesheets::{CssRuleType, Origin, UrlExtraData, layer_rule::LayerOrder},
    values::{
        generics::NonNegative,
        specified::{LengthPercentage, NoCalcLength, NoCalcPercentage},
    },
};
use style_traits::ParsingMode;

use crate::dom::native::Element;

use super::{
    presentational_hints::{
        synthesize_directionality_presentational_hint,
        synthesize_hidden_until_found_presentational_hint,
    },
    query::QueryElement,
};

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

// Mirrors the presentation attributes without element-specific SVG rules.
// These attributes participate in the author cascade as presentation hints:
// author rules and an inline style override them, while inheritance observes
// their parsed CSS value. Geometry and transform attributes are selected by
// `svg_presentation_property_name` because their spelling or applicability
// depends on the element.
const SVG_STYLE_PRESENTATION_ATTRIBUTES: &[&str] = &[
    "alignment-baseline",
    "baseline-shift",
    "buffered-rendering",
    "clip",
    "clip-path",
    "clip-rule",
    "color",
    "color-interpolation",
    "color-interpolation-filters",
    "color-rendering",
    "cursor",
    "direction",
    "display",
    "dominant-baseline",
    "fill",
    "fill-opacity",
    "fill-rule",
    "filter",
    "flood-color",
    "flood-opacity",
    "font-family",
    "font-size",
    "font-size-adjust",
    "font-stretch",
    "font-style",
    "font-variant",
    "font-weight",
    "image-rendering",
    "letter-spacing",
    "lighting-color",
    "marker-end",
    "marker-mid",
    "marker-start",
    "mask",
    "mask-type",
    "opacity",
    "overflow",
    "paint-order",
    "pointer-events",
    "shape-rendering",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-opacity",
    "stroke-width",
    "text-anchor",
    "text-decoration",
    "text-overflow",
    "text-rendering",
    "transform-origin",
    "unicode-bidi",
    "vector-effect",
    "visibility",
    "white-space",
    "word-spacing",
    "writing-mode",
];

const SVG_SPECIAL_PRESENTATION_ATTRIBUTES: &[&str] = &[
    "cx",
    "cy",
    "d",
    "gradientTransform",
    "height",
    "patternTransform",
    "r",
    "rx",
    "ry",
    "transform",
    "width",
    "x",
    "y",
];

/// Whether changing an attribute can change an SVG element's computed style
/// without any selector dependency on that attribute.
pub fn is_svg_presentation_attribute_name(name: &str) -> bool {
    SVG_STYLE_PRESENTATION_ATTRIBUTES.contains(&name)
        || SVG_SPECIAL_PRESENTATION_ATTRIBUTES.contains(&name)
}

impl QueryElement<'_> {
    pub(in crate::stylo) fn synthesize_presentational_hints<V>(&self, hints: &mut V)
    where
        V: Push<ApplicableDeclarationBlock>,
    {
        let element = self.element();
        let mut block = PropertyDeclarationBlock::new();
        if element.namespace() == SVG_NAMESPACE {
            let is_outermost_svg = element.local_name() == "svg"
                && self
                    .parent_element()
                    .is_none_or(|parent| parent.element().namespace() != SVG_NAMESPACE);
            if is_outermost_svg {
                append_root_svg_size_declarations(element, &mut block);
            }

            let base_url = self
                .host()
                .owner_document_handle(self.handle())
                .and_then(|document| self.host().document_base_url_for_handle(document));
            if let Some(base_url) = base_url {
                append_svg_style_presentation_declarations(
                    element,
                    &UrlExtraData::from(base_url),
                    self.read_quirks_mode(),
                    &mut block,
                );
            }
        } else if element.namespace() == HTML_NAMESPACE {
            if element.local_name() == "object" {
                append_html_object_dimension_declarations(element, &mut block);
            }
            if matches!(element.local_name(), "td" | "th") {
                append_html_table_cell_padding_declarations(*self, &mut block);
            }
        }

        if !block.is_empty() {
            hints.push(ApplicableDeclarationBlock::from_declarations(
                Arc::new(self.shared_lock().wrap(block)),
                CascadeLevel::new(CascadeOrigin::PresHints),
                LayerOrder::root(),
            ));
        }
        synthesize_hidden_until_found_presentational_hint(
            self.host(),
            self.handle(),
            self.shared_lock(),
            hints,
        );
        synthesize_directionality_presentational_hint(
            self.host(),
            self.handle(),
            self.shared_lock(),
            hints,
        );
    }
}

fn append_root_svg_size_declarations(element: &Element, block: &mut PropertyDeclarationBlock) {
    for (attribute, is_width) in [("width", true), ("height", false)] {
        let Some(value) = element.attribute(attribute) else {
            continue;
        };
        let Some(size) = parse_svg_size_attribute(value) else {
            continue;
        };
        use style::values::generics::length::Size;
        let size = Size::LengthPercentage(NonNegative(size));
        let declaration = if is_width {
            PropertyDeclaration::Width(size)
        } else {
            PropertyDeclaration::Height(size)
        };
        block.push(declaration, Importance::Normal);
    }
}

fn append_svg_style_presentation_declarations(
    element: &Element,
    url_data: &UrlExtraData,
    quirks_mode: QuirksMode,
    block: &mut PropertyDeclarationBlock,
) {
    for attribute in element.attributes() {
        if !attribute.namespace().is_empty() {
            continue;
        }
        let Some(property_name) = svg_presentation_property_name(element, attribute.local_name())
        else {
            continue;
        };
        let Ok(property) = PropertyId::parse_enabled_for_all_content(property_name) else {
            continue;
        };
        let mut svg_path_value = String::new();
        let value = if property_name == "d" {
            svg_path_value.push_str("path(");
            serialize_string(attribute.value(), &mut svg_path_value)
                .expect("serializing an SVG path into a String cannot fail");
            svg_path_value.push(')');
            svg_path_value.as_str()
        } else {
            attribute.value()
        };
        let mut declarations = SourcePropertyDeclaration::default();
        if parse_one_declaration_into(
            &mut declarations,
            property,
            value,
            Origin::Author,
            url_data,
            None,
            ParsingMode::ALLOW_UNITLESS_LENGTH | ParsingMode::ALLOW_ALL_NUMERIC_VALUES,
            quirks_mode,
            CssRuleType::Style,
        )
        .is_ok()
        {
            block.extend(declarations.drain(), Importance::Normal);
        }
    }
}

fn svg_presentation_property_name<'a>(
    element: &Element,
    attribute_name: &'a str,
) -> Option<&'a str> {
    // The timing `fill` attribute on SVG animation elements must not be
    // reinterpreted as the CSS fill presentation hint. Other presentation
    // attributes remain eligible even though they do not affect rendering.
    if attribute_name == "fill"
        && matches!(
            element.local_name(),
            "animate" | "animateMotion" | "animateTransform" | "set"
        )
    {
        return None;
    }

    if SVG_STYLE_PRESENTATION_ATTRIBUTES.contains(&attribute_name) {
        return Some(attribute_name);
    }

    let local_name = element.local_name();
    let applies = match attribute_name {
        "cx" | "cy" => matches!(local_name, "circle" | "ellipse"),
        "r" => local_name == "circle",
        "rx" | "ry" => matches!(local_name, "ellipse" | "rect"),
        "x" | "y" | "width" | "height" => {
            matches!(local_name, "foreignObject" | "image" | "rect" | "use")
        }
        "d" => local_name == "path",
        "transform" => is_svg_graphics_element(local_name),
        "patternTransform" => local_name == "pattern",
        "gradientTransform" => matches!(local_name, "linearGradient" | "radialGradient"),
        _ => false,
    };
    applies.then_some(match attribute_name {
        "patternTransform" | "gradientTransform" => "transform",
        _ => attribute_name,
    })
}

fn is_svg_graphics_element(local_name: &str) -> bool {
    matches!(
        local_name,
        "a" | "circle"
            | "defs"
            | "ellipse"
            | "foreignObject"
            | "g"
            | "image"
            | "line"
            | "path"
            | "polygon"
            | "polyline"
            | "rect"
            | "svg"
            | "symbol"
            | "switch"
            | "text"
            | "textPath"
            | "tspan"
            | "use"
    )
}

fn append_html_table_cell_padding_declarations(
    element: QueryElement<'_>,
    block: &mut PropertyDeclarationBlock,
) {
    // Blink's HTMLTableCellElement asks its nearest parent table for a shared
    // cell style. Cells without a parent table instead receive the equivalent
    // 1px fallback from the UA stylesheet.
    let mut ancestor = element.parent_element();
    let padding = loop {
        let Some(current) = ancestor else {
            return;
        };
        let native = current.element();
        if native.namespace() == HTML_NAMESPACE && native.local_name() == "table" {
            break parse_html_table_cell_padding(native.attribute("cellpadding"));
        }
        ancestor = current.parent_element();
    };

    // Chromium omits the shared declaration for zero rather than emitting
    // `padding: 0`. That distinction lets lower cascade origins remain
    // observable when the legacy attribute disables the default padding.
    if padding == 0 {
        return;
    }

    let padding = NonNegative(LengthPercentage::Length(NoCalcLength::from_px(f32::from(
        padding,
    ))));
    for declaration in [
        PropertyDeclaration::PaddingTop(padding.clone()),
        PropertyDeclaration::PaddingRight(padding.clone()),
        PropertyDeclaration::PaddingBottom(padding.clone()),
        PropertyDeclaration::PaddingLeft(padding),
    ] {
        block.push(declaration, Importance::Normal);
    }
}

fn append_html_object_dimension_declarations(
    element: &Element,
    block: &mut PropertyDeclarationBlock,
) {
    for (attribute, is_width) in [("width", true), ("height", false)] {
        let Some(value) = element.attribute(attribute) else {
            continue;
        };
        let Some(dimension) = parse_html_dimension_attribute(value) else {
            continue;
        };
        use style::values::generics::length::Size;
        let size = match dimension {
            HtmlDimension::Pixels(value) => LengthPercentage::Length(NoCalcLength::from_px(value)),
            HtmlDimension::Percentage(value) => {
                LengthPercentage::Percentage(NoCalcPercentage::new(value / 100.0))
            }
        };
        let size = Size::LengthPercentage(NonNegative(size));
        let declaration = if is_width {
            PropertyDeclaration::Width(size)
        } else {
            PropertyDeclaration::Height(size)
        };
        block.push(declaration, Importance::Normal);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum HtmlDimension {
    Pixels(f32),
    Percentage(f32),
}

/// Parses an HTML dimension value as a non-negative CSS pixel length or
/// percentage. HTML's legacy algorithm deliberately accepts trailing junk,
/// but requires the value itself to start with an ASCII digit after leading
/// ASCII whitespace.
fn parse_html_dimension_attribute(value: &str) -> Option<HtmlDimension> {
    let value = value.trim_start_matches(['\t', '\n', '\u{000C}', '\r', ' ']);
    let bytes = value.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }

    let integer_end = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    let mut number_end = integer_end;
    if bytes.get(integer_end) == Some(&b'.')
        && bytes.get(integer_end + 1).is_some_and(u8::is_ascii_digit)
    {
        number_end += 1;
        number_end += bytes[number_end..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
    }

    let number = value[..number_end]
        .parse::<f32>()
        .ok()
        .filter(|number| number.is_finite())?;
    if bytes.get(number_end) == Some(&b'%') {
        Some(HtmlDimension::Percentage(number))
    } else {
        Some(HtmlDimension::Pixels(number))
    }
}

/// Mirrors Blink's legacy `cellpadding` state: an absent or exactly empty
/// attribute keeps the historical 1px default; non-empty values use loose
/// signed-integer parsing and are clamped to `uint16_t`.
fn parse_html_table_cell_padding(value: Option<&str>) -> u16 {
    let Some(value) = value else {
        return 1;
    };
    if value.is_empty() {
        return 1;
    }

    parse_loose_i32(value)
        .unwrap_or(0)
        .clamp(0, i32::from(u16::MAX)) as u16
}

fn parse_loose_i32(value: &str) -> Option<i32> {
    let value = value.trim_start();
    let digits_start = usize::from(value.starts_with(['+', '-']));
    let digits_len = value[digits_start..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    (digits_len != 0)
        .then(|| &value[..digits_start + digits_len])
        .and_then(|number| number.parse().ok())
}

/// Parses the SVG 2 root `width`/`height` presentation attributes.
///
/// These are CSS `<length-percentage>` values, unlike legacy HTML dimension
/// attributes. Unitless numbers are SVG user units and therefore CSS pixels;
/// relative units such as `em`, `rem`, and viewport units remain specified
/// lengths here so Stylo resolves them in the element's real style context.
fn parse_svg_size_attribute(value: &str) -> Option<LengthPercentage> {
    let value = value.trim();
    if let Some(number) = value.strip_suffix('%') {
        let value = number.trim().parse::<f32>().ok()?;
        return (value.is_finite() && value >= 0.0)
            .then(|| LengthPercentage::Percentage(NoCalcPercentage::new(value / 100.0)));
    }

    // A CSS dimension has no whitespace between its number and unit. Taking
    // only a trailing alphabetic run leaves scientific notation such as
    // `1e3` intact because it ends in a digit.
    let number_len = value
        .trim_end_matches(|character: char| character.is_ascii_alphabetic())
        .len();
    let (number, unit) = value.split_at(number_len);
    let value = number
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)?;
    let length = if unit.is_empty() {
        NoCalcLength::from_px(value)
    } else {
        NoCalcLength::parse_dimension_with_flags(ParsingMode::DEFAULT, false, value, unit).ok()?
    };
    Some(LengthPercentage::Length(length))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_size_attribute_preserves_relative_units_for_stylo() {
        assert!(parse_svg_size_attribute("1em").is_some());
        assert!(parse_svg_size_attribute("1.5rem").is_some());
        assert!(parse_svg_size_attribute("24").is_some());
        assert!(parse_svg_size_attribute("50%").is_some());
        assert!(parse_svg_size_attribute("auto").is_none());
        assert!(parse_svg_size_attribute("-1em").is_none());
    }

    #[test]
    fn svg_attributes_are_classified_as_presentational() {
        for name in [
            "fill",
            "fill-opacity",
            "stroke",
            "stroke-width",
            "paint-order",
            "shape-rendering",
            "font-size-adjust",
            "text-overflow",
            "white-space",
            "width",
            "height",
            "x",
            "y",
            "cx",
            "cy",
            "r",
            "rx",
            "ry",
            "d",
            "transform",
            "patternTransform",
            "gradientTransform",
        ] {
            assert!(
                is_svg_presentation_attribute_name(name),
                "{name} must invalidate presentation hints when mutated"
            );
        }
        assert!(!is_svg_presentation_attribute_name("viewBox"));
    }

    #[test]
    fn svg_special_presentation_attributes_follow_element_scopes() {
        let element = |local_name: &str| {
            Element::new(
                local_name.to_owned(),
                SVG_NAMESPACE.to_owned(),
                None,
                Vec::new(),
            )
        };

        for local_name in ["foreignObject", "image", "rect", "use"] {
            let element = element(local_name);
            for attribute in ["x", "y", "width", "height"] {
                assert_eq!(
                    svg_presentation_property_name(&element, attribute),
                    Some(attribute),
                    "{attribute} on {local_name}"
                );
            }
        }
        for local_name in ["g", "symbol"] {
            let element = element(local_name);
            assert_eq!(
                svg_presentation_property_name(&element, "transform"),
                Some("transform")
            );
        }
        assert_eq!(
            svg_presentation_property_name(&element("pattern"), "patternTransform"),
            Some("transform")
        );
        assert_eq!(
            svg_presentation_property_name(&element("linearGradient"), "gradientTransform"),
            Some("transform")
        );
        assert_eq!(
            svg_presentation_property_name(&element("radialGradient"), "gradientTransform"),
            Some("transform")
        );

        let group = element("g");
        for attribute in ["x", "y", "width", "height"] {
            assert_eq!(svg_presentation_property_name(&group, attribute), None);
        }
        for local_name in ["pattern", "linearGradient", "radialGradient"] {
            assert_eq!(
                svg_presentation_property_name(&element(local_name), "transform"),
                None
            );
        }
        let animate = element("animate");
        assert_eq!(svg_presentation_property_name(&animate, "fill"), None);
        assert_eq!(
            svg_presentation_property_name(&animate, "stroke"),
            Some("stroke")
        );
    }

    #[test]
    fn html_table_cell_padding_matches_blink_legacy_parsing() {
        assert_eq!(parse_html_table_cell_padding(None), 1);
        assert_eq!(parse_html_table_cell_padding(Some("")), 1);
        assert_eq!(parse_html_table_cell_padding(Some("0")), 0);
        assert_eq!(parse_html_table_cell_padding(Some("  +12px")), 12);
        assert_eq!(parse_html_table_cell_padding(Some("-3")), 0);
        assert_eq!(parse_html_table_cell_padding(Some("70000")), u16::MAX);
        assert_eq!(parse_html_table_cell_padding(Some("not-a-number")), 0);
        assert_eq!(parse_html_table_cell_padding(Some("   ")), 0);
        assert_eq!(parse_html_table_cell_padding(Some("2147483648")), 0);
    }

    #[test]
    fn html_dimension_attribute_uses_legacy_dimension_parsing() {
        assert_eq!(
            parse_html_dimension_attribute("  100"),
            Some(HtmlDimension::Pixels(100.0))
        );
        assert_eq!(
            parse_html_dimension_attribute("12.5px"),
            Some(HtmlDimension::Pixels(12.5))
        );
        assert_eq!(
            parse_html_dimension_attribute("25.5%ignored"),
            Some(HtmlDimension::Percentage(25.5))
        );
        assert_eq!(
            parse_html_dimension_attribute("1.%"),
            Some(HtmlDimension::Pixels(1.0))
        );
        assert_eq!(parse_html_dimension_attribute("+10"), None);
        assert_eq!(parse_html_dimension_attribute(".5"), None);
        assert_eq!(parse_html_dimension_attribute("auto"), None);
    }
}
