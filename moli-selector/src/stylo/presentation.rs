// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Presentational hints belong at the Stylo adapter boundary. Keeping legacy
// HTML attributes and SVG presentation attributes here lets the normal
// cascade, inheritance, and relative-length resolver own the result instead
// of teaching layout about authored attribute strings.

use selectors::{Element as SelectorsElement, sink::Push};
use style::{
    applicable_declarations::ApplicableDeclarationBlock,
    properties::{Importance, PropertyDeclaration, PropertyDeclarationBlock},
    rule_tree::{CascadeLevel, CascadeOrigin},
    servo_arc::Arc,
    shared_lock::SharedRwLock,
    stylesheets::layer_rule::LayerOrder,
    values::{
        generics::NonNegative,
        specified::{AspectRatio, LengthPercentage, NoCalcLength, NoCalcPercentage},
    },
};
use style_traits::ParsingMode;

use crate::dom::native::Element;

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// The two Stylo element adapters deliberately share presentation-hint
/// synthesis. This keeps ancestor-dependent HTML hints identical during full
/// traversal and query-only style resolution.
pub(super) trait PresentationalHintElement: SelectorsElement + Copy {
    fn native_element(&self) -> &Element;
}

pub(super) fn synthesize_presentational_hints<E, V>(
    element: E,
    shared_lock: &SharedRwLock,
    hints: &mut V,
) where
    E: PresentationalHintElement,
    V: Push<ApplicableDeclarationBlock>,
{
    synthesize_svg_root_size(element.native_element(), shared_lock, hints);
    synthesize_html_replaced_size(element.native_element(), shared_lock, hints);
    synthesize_html_table_cell_style(element, shared_lock, hints);
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum HtmlDimension {
    Absolute(f32),
    Percentage(f32),
    Relative,
}

/// Map legacy HTML dimensions into the presentation-hint cascade.
///
/// Images and videos expose both width/height declarations and an `auto`
/// aspect-ratio hint. Plug-in/frame owners expose only the dimensions. Canvas
/// dimensions define its intrinsic bitmap rather than CSS width/height, but a
/// pair still contributes an `auto` aspect ratio. This is the same ownership
/// split used by Blink's element-specific
/// `CollectStyleForPresentationAttribute` implementations.
fn synthesize_html_replaced_size<V>(element: &Element, shared_lock: &SharedRwLock, hints: &mut V)
where
    V: Push<ApplicableDeclarationBlock>,
{
    if element.namespace() != HTML_NAMESPACE {
        return;
    }

    let name = element.local_name();
    let input_maps_dimensions = name == "input"
        && element.attribute("type").is_some_and(|value| {
            value.eq_ignore_ascii_case("image") || value.eq_ignore_ascii_case("hidden")
        });
    let maps_dimensions =
        matches!(name, "img" | "video" | "iframe" | "object" | "embed") || input_maps_dimensions;
    let maps_ratio = matches!(name, "img" | "video") || input_maps_dimensions;

    if name == "canvas" {
        let ratio = element
            .attribute("width")
            .and_then(parse_html_non_negative_integer)
            .zip(
                element
                    .attribute("height")
                    .and_then(parse_html_non_negative_integer),
            )
            .map(|(width, height)| {
                PropertyDeclaration::AspectRatio(Box::new(AspectRatio::from_mapped_ratio(
                    width as f32,
                    height as f32,
                )))
            });
        if let Some(ratio) = ratio {
            push_presentational_declarations(shared_lock, hints, [ratio]);
        }
        return;
    }
    if !maps_dimensions {
        return;
    }

    let width = element.attribute("width").and_then(parse_html_dimension);
    let height = element.attribute("height").and_then(parse_html_dimension);
    let mut declarations = Vec::with_capacity(3);
    if let Some(value) = width.and_then(html_dimension_length_percentage) {
        use style::values::generics::length::Size;
        declarations.push(PropertyDeclaration::Width(Size::LengthPercentage(
            NonNegative(value),
        )));
    }
    if let Some(value) = height.and_then(html_dimension_length_percentage) {
        use style::values::generics::length::Size;
        declarations.push(PropertyDeclaration::Height(Size::LengthPercentage(
            NonNegative(value),
        )));
    }
    if maps_ratio
        && let (Some(HtmlDimension::Absolute(width)), Some(HtmlDimension::Absolute(height))) =
            (width, height)
    {
        declarations.push(PropertyDeclaration::AspectRatio(Box::new(
            AspectRatio::from_mapped_ratio(width, height),
        )));
    }
    push_presentational_declarations(shared_lock, hints, declarations);
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

/// Blink-compatible parsing for legacy HTML dimension values. The numeric
/// prefix is accepted with trailing garbage; a directly following `%` or `*`
/// selects percentage or obsolete relative syntax respectively.
fn parse_html_dimension(value: &str) -> Option<HtmlDimension> {
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
    let number = value[..number_len].parse::<f64>().ok()?;
    if !number.is_finite() {
        return None;
    }
    let number = number.min(f64::from(f32::MAX)) as f32;
    match bytes.get(number_len) {
        Some(b'%') => Some(HtmlDimension::Percentage(number)),
        Some(b'*') => Some(HtmlDimension::Relative),
        _ => Some(HtmlDimension::Absolute(number)),
    }
}

fn parse_html_non_negative_integer(value: &str) -> Option<u32> {
    let value = value.trim_start_matches([' ', '\t', '\n', '\r', '\u{000c}']);
    let (value, negative) = if let Some(value) = value.strip_prefix('+') {
        (value, false)
    } else if let Some(value) = value.strip_prefix('-') {
        (value, true)
    } else {
        (value, false)
    };
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    let parsed = (digits != 0)
        .then(|| &value[..digits])
        .and_then(|value| value.parse().ok())?;
    (!negative || parsed == 0).then_some(parsed)
}

fn synthesize_svg_root_size<V>(element: &Element, shared_lock: &SharedRwLock, hints: &mut V)
where
    V: Push<ApplicableDeclarationBlock>,
{
    if element.namespace() != SVG_NAMESPACE || element.local_name() != "svg" {
        return;
    }

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
        push_presentational_declarations(shared_lock, hints, [declaration]);
    }
}

fn synthesize_html_table_cell_style<E, V>(element: E, shared_lock: &SharedRwLock, hints: &mut V)
where
    E: PresentationalHintElement,
    V: Push<ApplicableDeclarationBlock>,
{
    let native = element.native_element();
    if native.namespace() != HTML_NAMESPACE || !matches!(native.local_name(), "td" | "th") {
        return;
    }

    // Blink's HTMLTableCellElement asks its nearest parent table for a shared
    // cell style. Cells without a parent table instead receive the equivalent
    // 1px fallback from the UA stylesheet.
    let mut ancestor = element.parent_element();
    let padding = loop {
        let Some(current) = ancestor else {
            return;
        };
        let native = current.native_element();
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
    push_presentational_declarations(
        shared_lock,
        hints,
        [
            PropertyDeclaration::PaddingTop(padding.clone()),
            PropertyDeclaration::PaddingRight(padding.clone()),
            PropertyDeclaration::PaddingBottom(padding.clone()),
            PropertyDeclaration::PaddingLeft(padding),
        ],
    );
}

fn push_presentational_declarations<V>(
    shared_lock: &SharedRwLock,
    hints: &mut V,
    declarations: impl IntoIterator<Item = PropertyDeclaration>,
) where
    V: Push<ApplicableDeclarationBlock>,
{
    let mut block = PropertyDeclarationBlock::new();
    for declaration in declarations {
        let _ = block.push(declaration, Importance::Normal);
    }
    if block.is_empty() {
        return;
    }
    hints.push(ApplicableDeclarationBlock::from_declarations(
        Arc::new(shared_lock.wrap(block)),
        CascadeLevel::new(CascadeOrigin::PresHints),
        LayerOrder::root(),
    ));
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
    fn html_dimension_parser_matches_blink_numeric_prefix_rules() {
        assert_eq!(
            parse_html_dimension("  10"),
            Some(HtmlDimension::Absolute(10.0))
        );
        assert_eq!(
            parse_html_dimension("10.5px"),
            Some(HtmlDimension::Absolute(10.5))
        );
        assert_eq!(
            parse_html_dimension("10.%"),
            Some(HtmlDimension::Percentage(10.0))
        );
        assert_eq!(
            parse_html_dimension("10%garbage"),
            Some(HtmlDimension::Percentage(10.0))
        );
        assert_eq!(parse_html_dimension("10*"), Some(HtmlDimension::Relative));
        assert_eq!(
            parse_html_dimension("10e10"),
            Some(HtmlDimension::Absolute(10.0))
        );
        assert_eq!(parse_html_dimension("+10"), None);
        assert_eq!(parse_html_dimension(".5"), None);
        assert_eq!(parse_html_dimension(""), None);
    }

    #[test]
    fn canvas_ratio_integer_parser_keeps_invalid_values_out_of_the_cascade() {
        assert_eq!(parse_html_non_negative_integer("  +12px"), Some(12));
        assert_eq!(parse_html_non_negative_integer("0"), Some(0));
        assert_eq!(parse_html_non_negative_integer("-0garbage"), Some(0));
        assert_eq!(parse_html_non_negative_integer("-1"), None);
        assert_eq!(parse_html_non_negative_integer("not-a-number"), None);
        assert_eq!(parse_html_non_negative_integer("4294967296"), None);
    }
}
