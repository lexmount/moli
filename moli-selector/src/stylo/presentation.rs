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
        specified::{LengthPercentage, NoCalcLength, NoCalcPercentage},
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
    synthesize_html_table_cell_style(element, shared_lock, hints);
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
}
