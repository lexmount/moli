// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Presentational hints belong at the Stylo adapter boundary. Keeping legacy
// HTML attributes and SVG presentation attributes here lets the normal
// cascade, inheritance, and relative-length resolver own the result instead
// of teaching layout about authored attribute strings.

use moli_dom::html_microsyntax::parse_non_negative_integer;
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
        specified::{AspectRatio, LengthPercentage, NoCalcLength, NoCalcPercentage},
    },
};
use style_traits::{ParsingMode, ToCss};

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

#[derive(Clone, Copy, Debug, PartialEq)]
enum HtmlDimension {
    Absolute(f32),
    Percentage(f32),
    Relative,
}

/// Maps replaced-element HTML dimensions into the presentation-hint cascade.
///
/// Image-like elements expose width, height, and a mapped `auto <ratio>`.
/// Frame and plug-in owners expose only width and height. Canvas dimensions
/// instead define its intrinsic bitmap; only a valid pair contributes a
/// mapped ratio here. This follows Blink's element-specific
/// `CollectStyleForPresentationAttribute` ownership split.
fn append_html_replaced_size_declarations(element: &Element, block: &mut PropertyDeclarationBlock) {
    let name = element.local_name();
    let input_maps_dimensions = name == "input"
        && element.attribute("type").is_some_and(|value| {
            value.eq_ignore_ascii_case("image") || value.eq_ignore_ascii_case("hidden")
        });
    let maps_dimensions =
        matches!(name, "img" | "video" | "iframe" | "object" | "embed") || input_maps_dimensions;
    let maps_ratio = matches!(name, "img" | "video") || input_maps_dimensions;

    if name == "canvas" {
        if let Some((width, height)) = element
            .attribute("width")
            .and_then(parse_non_negative_integer)
            .zip(
                element
                    .attribute("height")
                    .and_then(parse_non_negative_integer),
            )
        {
            let _ = block.push(
                PropertyDeclaration::AspectRatio(Box::new(AspectRatio::from_mapped_ratio(
                    width as f32,
                    height as f32,
                ))),
                Importance::Normal,
            );
        }
        return;
    }
    if !maps_dimensions {
        return;
    }

    let width = element
        .attribute("width")
        .and_then(parse_html_dimension_value);
    let height = element
        .attribute("height")
        .and_then(parse_html_dimension_value);
    if let Some(value) = width.and_then(html_dimension_length_percentage) {
        use style::values::generics::length::Size;
        let _ = block.push(
            PropertyDeclaration::Width(Size::LengthPercentage(NonNegative(value))),
            Importance::Normal,
        );
    }
    if let Some(value) = height.and_then(html_dimension_length_percentage) {
        use style::values::generics::length::Size;
        let _ = block.push(
            PropertyDeclaration::Height(Size::LengthPercentage(NonNegative(value))),
            Importance::Normal,
        );
    }
    if maps_ratio
        && let (Some(HtmlDimension::Absolute(width)), Some(HtmlDimension::Absolute(height))) =
            (width, height)
    {
        let _ = block.push(
            PropertyDeclaration::AspectRatio(Box::new(AspectRatio::from_mapped_ratio(
                width, height,
            ))),
            Importance::Normal,
        );
    }
}

fn html_dimension_length_percentage(dimension: HtmlDimension) -> Option<LengthPercentage> {
    match dimension {
        HtmlDimension::Absolute(value) => {
            Some(LengthPercentage::Length(NoCalcLength::from_px(value)))
        }
        HtmlDimension::Percentage(value) => Some(LengthPercentage::Percentage(
            NoCalcPercentage::new(value / 100.0),
        )),
        HtmlDimension::Relative => None,
    }
}

/// Parses the numeric prefix used by Blink's legacy HTML dimension rules.
fn parse_html_dimension_value(value: &str) -> Option<HtmlDimension> {
    let value = value.trim_start_matches([' ', '\t', '\n', '\r', '\u{000c}']);
    let bytes = value.as_bytes();
    let integer_len = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if integer_len == 0 {
        return None;
    }

    let mut number_len = integer_len;
    if bytes.get(number_len) == Some(&b'.') {
        number_len += 1;
        number_len += bytes[number_len..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
    }
    // Blink clamps the parsed double before storing it as a CSS value. The
    // grammar above cannot produce NaN, while a very long digit run can
    // overflow to infinity, so saturate it instead of rejecting an otherwise
    // valid legacy dimension.
    let number = value[..number_len]
        .parse::<f64>()
        .ok()?
        .min(f64::from(f32::MAX)) as f32;
    match bytes.get(number_len) {
        Some(b'%') => Some(HtmlDimension::Percentage(number)),
        Some(b'*') => Some(HtmlDimension::Relative),
        _ => Some(HtmlDimension::Absolute(number)),
    }
}

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
        match element.namespace() {
            SVG_NAMESPACE => {
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
            }
            HTML_NAMESPACE => {
                append_html_replaced_size_declarations(element, &mut block);
                append_html_table_presentation_declarations(*self, &mut block);
            }
            _ => return,
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

fn append_html_table_presentation_declarations(
    element: QueryElement<'_>,
    block: &mut PropertyDeclarationBlock,
) {
    let native = element.element();
    let local_name = native.local_name();
    if local_name == "table" {
        if let Some(width) = native.attribute("width").and_then(parse_html_table_width) {
            use style::values::generics::length::Size;
            block.push(
                PropertyDeclaration::Width(Size::LengthPercentage(NonNegative(width))),
                Importance::Normal,
            );
        }

        if let Some(spacing) = native
            .attribute("cellspacing")
            .filter(|value| !value.is_empty())
            .and_then(parse_html_table_cell_spacing)
        {
            append_parsed_presentation_declaration(
                element,
                "border-spacing",
                &spacing.to_css_string(),
                block,
            );
        }

        if let Some(vertical_align) = native.attribute("valign").filter(|value| !value.is_empty()) {
            append_parsed_presentation_declaration(
                element,
                "vertical-align",
                vertical_align,
                block,
            );
        }
    }

    if matches!(
        local_name,
        "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
    ) && let Some(color) = native
        .attribute("bgcolor")
        .and_then(parse_html_legacy_color)
    {
        append_parsed_presentation_declaration(element, "background-color", &color, block);
    }

    if matches!(local_name, "thead" | "tbody" | "tfoot" | "tr" | "td" | "th") {
        append_html_table_part_alignment_declarations(element, block);
    }

    if matches!(local_name, "td" | "th") {
        append_html_table_cell_padding_declarations(element, block);
    }
}

fn append_html_table_part_alignment_declarations(
    element: QueryElement<'_>,
    block: &mut PropertyDeclarationBlock,
) {
    let native = element.element();
    if let Some(vertical_align) = native.attribute("valign") {
        let vertical_align = match vertical_align.to_ascii_lowercase().as_str() {
            "top" => "top",
            "middle" => "middle",
            "bottom" => "bottom",
            "baseline" => "baseline",
            _ => vertical_align,
        };
        append_parsed_presentation_declaration(element, "vertical-align", vertical_align, block);
    }

    if let Some(text_align) = native.attribute("align") {
        let text_align = match text_align.to_ascii_lowercase().as_str() {
            "middle" | "center" => "-moz-center",
            "absmiddle" => "center",
            "left" => "-moz-left",
            "right" => "-moz-right",
            _ => text_align,
        };
        append_parsed_presentation_declaration(element, "text-align", text_align, block);
    }
}

fn append_parsed_presentation_declaration(
    element: QueryElement<'_>,
    property_name: &str,
    value: &str,
    block: &mut PropertyDeclarationBlock,
) {
    let Some(base_url) = element
        .host()
        .owner_document_handle(element.handle())
        .and_then(|document| element.host().document_base_url_for_handle(document))
    else {
        return;
    };
    let Ok(property) = PropertyId::parse_enabled_for_all_content(property_name) else {
        return;
    };
    let mut declarations = SourcePropertyDeclaration::default();
    if parse_one_declaration_into(
        &mut declarations,
        property,
        value,
        Origin::Author,
        &UrlExtraData::from(base_url),
        None,
        ParsingMode::DEFAULT,
        element.read_quirks_mode(),
        CssRuleType::Style,
    )
    .is_ok()
    {
        block.extend(declarations.drain(), Importance::Normal);
    }
}

/// Applies the table `width` policy from Blink's `AddHTMLLengthToStyle`:
/// percentages are accepted, while zero and relative dimensions are omitted.
fn parse_html_table_width(value: &str) -> Option<LengthPercentage> {
    match parse_html_dimension_value(value)? {
        HtmlDimension::Absolute(0.0) | HtmlDimension::Percentage(0.0) | HtmlDimension::Relative => {
            None
        }
        dimension => html_dimension_length_percentage(dimension),
    }
}

/// Applies the table `cellspacing` policy: zero is meaningful, but percentage
/// and relative dimensions do not become a border-spacing declaration.
fn parse_html_table_cell_spacing(value: &str) -> Option<LengthPercentage> {
    match parse_html_dimension_value(value)? {
        HtmlDimension::Absolute(value) => {
            html_dimension_length_percentage(HtmlDimension::Absolute(value))
        }
        HtmlDimension::Percentage(_) | HtmlDimension::Relative => None,
    }
}

/// Parses an HTML legacy color and returns an opaque CSS hex color.
///
/// This follows Blink's `ParseColorWithLegacyRules`: standard three/six digit
/// hex and named colors win, while other non-empty values use HTML's historical
/// replacement-and-component algorithm.
fn parse_html_legacy_color(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let value =
        value.trim_matches(|character| matches!(character, '\t' | '\n' | '\u{000c}' | '\r' | ' '));
    if value.eq_ignore_ascii_case("transparent") {
        return None;
    }

    if let Some(color) = parse_short_or_long_hex_color(value) {
        return Some(color);
    }
    if let Ok((red, green, blue)) = cssparser::color::parse_named_color(value) {
        return Some(format!("#{red:02x}{green:02x}{blue:02x}"));
    }

    let mut digits = Vec::with_capacity(130);
    let value = value.strip_prefix('#').unwrap_or(value);
    for character in value.chars() {
        for unit in 0..character.len_utf16() {
            if digits.len() == 128 {
                break;
            }
            digits.push(if unit == 0 && character.is_ascii_hexdigit() {
                character as u8
            } else {
                b'0'
            });
        }
        if digits.len() == 128 {
            break;
        }
    }
    if digits.is_empty() {
        return Some("#000000".to_owned());
    }

    digits.extend_from_slice(b"00");
    if digits.len() < 6 {
        return Some(format!(
            "#{:02x}{:02x}{:02x}",
            hex_nibble(digits[0]),
            hex_nibble(digits[1]),
            hex_nibble(digits[2]),
        ));
    }

    let component_length = digits.len() / 3;
    let search_window = component_length.min(8);
    let mut red = component_length - search_window;
    let mut green = component_length * 2 - search_window;
    let mut blue = component_length * 3 - search_window;
    while digits[red] == b'0'
        && digits[green] == b'0'
        && digits[blue] == b'0'
        && component_length - red > 2
    {
        red += 1;
        green += 1;
        blue += 1;
    }
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        hex_byte(digits[red], digits[red + 1]),
        hex_byte(digits[green], digits[green + 1]),
        hex_byte(digits[blue], digits[blue + 1]),
    ))
}

fn parse_short_or_long_hex_color(value: &str) -> Option<String> {
    let digits = value.strip_prefix('#')?.as_bytes();
    match digits {
        [red, green, blue] if digits.iter().all(u8::is_ascii_hexdigit) => Some(format!(
            "#{0:01x}{0:01x}{1:01x}{1:01x}{2:01x}{2:01x}",
            hex_nibble(*red),
            hex_nibble(*green),
            hex_nibble(*blue),
        )),
        [
            red_high,
            red_low,
            green_high,
            green_low,
            blue_high,
            blue_low,
        ] if digits.iter().all(u8::is_ascii_hexdigit) => Some(format!(
            "#{:02x}{:02x}{:02x}",
            hex_byte(*red_high, *red_low),
            hex_byte(*green_high, *green_low),
            hex_byte(*blue_high, *blue_low),
        )),
        _ => None,
    }
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}

fn hex_byte(high: u8, low: u8) -> u8 {
    hex_nibble(high) * 16 + hex_nibble(low)
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

    #[test]
    fn html_table_dimensions_match_blink_legacy_parsing() {
        assert_eq!(
            parse_html_table_width("85%"),
            Some(LengthPercentage::Percentage(NoCalcPercentage::new(0.85)))
        );
        assert_eq!(
            parse_html_table_cell_spacing(" 10foo"),
            Some(LengthPercentage::Length(NoCalcLength::from_px(10.0)))
        );
        assert_eq!(
            parse_html_table_width("10.%"),
            Some(LengthPercentage::Percentage(NoCalcPercentage::new(0.1)))
        );
        assert!(parse_html_table_width("+10").is_none());
        assert!(parse_html_table_width(".5").is_none());
        assert!(parse_html_table_cell_spacing("50%").is_none());
        assert!(parse_html_table_width("0").is_none());
        assert!(parse_html_table_width("10*").is_none());
        assert_eq!(
            parse_html_table_width("10 *"),
            Some(LengthPercentage::Length(NoCalcLength::from_px(10.0)))
        );
    }

    #[test]
    fn html_table_colors_match_blink_legacy_parsing() {
        assert_eq!(
            parse_html_legacy_color("#f6f6ef").as_deref(),
            Some("#f6f6ef")
        );
        assert_eq!(parse_html_legacy_color("#f60").as_deref(), Some("#ff6600"));
        assert_eq!(parse_html_legacy_color("red").as_deref(), Some("#ff0000"));
        assert_eq!(
            parse_html_legacy_color("chucknorris").as_deref(),
            Some("#c00000")
        );
        assert_eq!(parse_html_legacy_color("transparent"), None);
        assert_eq!(parse_html_legacy_color(""), None);
    }

    #[test]
    fn html_dimension_parser_matches_blink_numeric_prefix_rules() {
        assert_eq!(
            parse_html_dimension_value("  10"),
            Some(HtmlDimension::Absolute(10.0))
        );
        assert_eq!(
            parse_html_dimension_value("10.5px"),
            Some(HtmlDimension::Absolute(10.5))
        );
        assert_eq!(
            parse_html_dimension_value("10.%"),
            Some(HtmlDimension::Percentage(10.0))
        );
        assert_eq!(
            parse_html_dimension_value("10%garbage"),
            Some(HtmlDimension::Percentage(10.0))
        );
        assert_eq!(
            parse_html_dimension_value("10*"),
            Some(HtmlDimension::Relative)
        );
        assert_eq!(
            parse_html_dimension_value("10e10"),
            Some(HtmlDimension::Absolute(10.0))
        );
        assert_eq!(parse_html_dimension_value("+10"), None);
        assert_eq!(parse_html_dimension_value(".5"), None);
        assert_eq!(parse_html_dimension_value(""), None);
        assert_eq!(
            parse_html_dimension_value(&"9".repeat(400)),
            Some(HtmlDimension::Absolute(f32::MAX))
        );
    }
}
