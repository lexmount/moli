// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Presentational hints belong at the Stylo adapter boundary. Keeping legacy
// HTML attributes and SVG presentation attributes here lets the normal
// cascade, inheritance, and relative-length resolver own the result instead
// of teaching layout about authored attribute strings.

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

use super::query::QueryElement;

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

// Mirrors Blink's CSSPropertyIdForSVGAttributeName allowlist. These attributes
// participate in the author cascade as presentation hints: author rules and an
// inline style override them, while inheritance observes their parsed CSS
// value. Geometry attributes such as the root SVG width/height are handled
// separately below because they have element-specific SVG parsing rules.
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
    "text-rendering",
    "transform-origin",
    "unicode-bidi",
    "vector-effect",
    "visibility",
    "word-spacing",
    "writing-mode",
];

/// Whether changing an attribute can change an SVG element's computed style
/// without any selector dependency on that attribute.
pub fn is_svg_presentation_attribute_name(name: &str) -> bool {
    matches!(name, "width" | "height") || SVG_STYLE_PRESENTATION_ATTRIBUTES.contains(&name)
}

impl QueryElement<'_> {
    pub(in crate::stylo) fn synthesize_presentational_hints<V>(&self, hints: &mut V)
    where
        V: Push<ApplicableDeclarationBlock>,
    {
        // Stylo collects presentation hints before checking
        // `AuthorStylesEnabled`, so the adapter must apply the same Document
        // policy explicitly. Otherwise SVG attributes such as `fill` and
        // legacy HTML attributes such as `cellpadding` leak into an
        // author-style-disabled cascade.
        if !self.author_styles_enabled() {
            return;
        }
        let element = self.element();
        let mut block = PropertyDeclarationBlock::new();
        if element.namespace() == SVG_NAMESPACE {
            if element.local_name() == "svg" {
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
        } else if element.namespace() == HTML_NAMESPACE
            && matches!(element.local_name(), "td" | "th")
        {
            append_html_table_cell_padding_declarations(*self, &mut block);
        } else {
            return;
        }

        if !block.is_empty() {
            hints.push(ApplicableDeclarationBlock::from_declarations(
                Arc::new(self.shared_lock().wrap(block)),
                CascadeLevel::new(CascadeOrigin::PresHints),
                LayerOrder::root(),
            ));
        }
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
        if !attribute.namespace().is_empty()
            || !SVG_STYLE_PRESENTATION_ATTRIBUTES.contains(&attribute.local_name())
        {
            continue;
        }
        let Ok(property) = PropertyId::parse_enabled_for_all_content(attribute.local_name()) else {
            continue;
        };
        let mut declarations = SourcePropertyDeclaration::default();
        if parse_one_declaration_into(
            &mut declarations,
            property,
            attribute.value(),
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
    fn svg_paint_attributes_are_classified_as_presentational() {
        for name in [
            "fill",
            "fill-opacity",
            "stroke",
            "stroke-width",
            "paint-order",
            "shape-rendering",
            "width",
            "height",
        ] {
            assert!(
                is_svg_presentation_attribute_name(name),
                "{name} must invalidate presentation hints when mutated"
            );
        }
        assert!(!is_svg_presentation_attribute_name("viewBox"));
        assert!(!is_svg_presentation_attribute_name("d"));
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
}
