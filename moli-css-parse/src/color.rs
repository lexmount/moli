//! CSS color parsing helpers shared by HTML and CSS observable surfaces.

use cssparser::Parser;
use style::{
    color::{AbsoluteColor, ColorSpace},
    context::QuirksMode,
    custom_properties::AttrTaint,
    parser::{Parse, ParserContext},
    stylesheets::{CssRuleType, Origin, UrlExtraData},
    values::specified::color::{Color, ColorScheme, ColorSchemeFlags, SystemColor},
};
use style_traits::{ParsingMode, ToCss};

/// Parse a Canvas color with HTML-compatible serialization. Preserve modern
/// color spaces, and resolve `currentcolor` at assignment time against the
/// canvas element's computed color (or black for an OffscreenCanvas).
pub fn parse_css_color_for_canvas(value: &str, current_color: Option<&str>) -> Option<CanvasColor> {
    let specified = parse_specified_color(value)?;
    let current = current_color
        .and_then(parse_specified_color)
        .and_then(|color| absolute_color(color, &AbsoluteColor::BLACK))
        .unwrap_or(AbsoluteColor::BLACK);
    Some(CanvasColor {
        absolute: absolute_color(specified, &current)?,
    })
}

/// Canvas retains the parsed color independently from its CSS serialization.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasColor {
    pub absolute: AbsoluteColor,
}

impl CanvasColor {
    pub fn serialize(&self) -> String {
        if self.absolute.is_legacy_syntax() {
            let legacy = self.absolute.into_srgb_legacy();
            if legacy.alpha == 1.0 {
                let [red, green, blue, _] = legacy.to_nscolor().to_le_bytes();
                return format!("#{red:02x}{green:02x}{blue:02x}");
            }
            return legacy.to_css_string();
        }
        self.absolute.to_css_string()
    }
}

/// Parse a CSS color and serialize its opaque sRGB channels as lowercase
/// `#rrggbb`.
///
/// This matches the HTML-compatible value mode of `<input type=color>`:
/// alpha is discarded, out-of-gamut channels are clipped during sRGB
/// serialization, `currentcolor` resolves to black, and system colors use the
/// headless Servo light palette.
pub fn parse_css_color_to_opaque_srgb_hex(value: &str) -> Option<String> {
    let [red, green, blue, _alpha] = parse_css_color_to_srgb_bytes(value)?;
    Some(format!("#{red:02x}{green:02x}{blue:02x}"))
}

/// Parse a CSS color and return clipped, unpremultiplied sRGB bytes.
///
/// This is the shared lower-level form used by protocol surfaces that must
/// preserve alpha instead of applying HTML `<input type=color>`'s opaque
/// serialization.
pub fn parse_css_color_to_srgb_bytes(value: &str) -> Option<[u8; 4]> {
    let absolute = parse_css_color_to_absolute(value)?;
    Some(absolute_srgb_bytes(absolute))
}

/// Parse without clipping RGB coordinates, so a native color-managed consumer
/// can choose its own destination color space and storage precision.
pub fn parse_css_color_to_absolute(value: &str) -> Option<AbsoluteColor> {
    absolute_color(parse_specified_color(value)?, &AbsoluteColor::BLACK)
}

fn absolute_color(color: Color, current: &AbsoluteColor) -> Option<AbsoluteColor> {
    Some(match color {
        Color::CurrentColor => *current,
        Color::System(system) => system_color_absolute(system),
        color => color
            .to_computed_color(None)
            .ok()?
            .resolve_to_absolute(current),
    })
}

/// Resolve a CSS system-color keyword through the headless Servo light
/// palette.
pub fn css_system_color_srgb(value: &str) -> Option<(u8, u8, u8)> {
    let Color::System(system) = parse_specified_color(value)? else {
        return None;
    };
    let [red, green, blue, _alpha] = absolute_srgb_bytes(system_color_absolute(system));
    Some((red, green, blue))
}

/// Parse the CSS `color-scheme` grammar and return its known-scheme flags.
///
/// A successful value with no flags is the valid `normal` keyword. Custom
/// schemes are preserved by Stylo's validation but do not set a known flag.
pub fn parse_css_color_scheme_flags(value: &str) -> Option<ColorSchemeFlags> {
    let url_data = UrlExtraData::from(url::Url::parse("about:blank").ok()?);
    let context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Style),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Default::default(),
        None,
        None,
        AttrTaint::default(),
    );
    let input = value;
    Parser::new(input)
        .parse_entirely(|input| ColorScheme::parse(&context, input))
        .ok()
        .map(|scheme| scheme.bits)
}

fn parse_specified_color(value: &str) -> Option<Color> {
    let url_data = UrlExtraData::from(url::Url::parse("about:blank").ok()?);
    let context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Style),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Default::default(),
        None,
        None,
        AttrTaint::default(),
    );
    let input = value;
    Parser::new(input)
        .parse_entirely(|input| Color::parse(&context, input))
        .ok()
}

fn system_color_absolute(system: SystemColor) -> AbsoluteColor {
    let (red, green, blue) = match system {
        SystemColor::Accentcolor | SystemColor::Selecteditem => (0, 102, 204),
        SystemColor::Accentcolortext | SystemColor::Selecteditemtext => (255, 255, 255),
        SystemColor::Activetext => (238, 0, 0),
        SystemColor::Linktext => (0, 0, 238),
        SystemColor::Visitedtext => (85, 26, 139),
        SystemColor::Buttonborder
        | SystemColor::Activeborder
        | SystemColor::Inactiveborder
        | SystemColor::Threeddarkshadow
        | SystemColor::Threedshadow
        | SystemColor::Windowframe => (169, 169, 169),
        SystemColor::Buttonface
        | SystemColor::Buttonhighlight
        | SystemColor::Buttonshadow
        | SystemColor::Threedface
        | SystemColor::Threedhighlight
        | SystemColor::Threedlightshadow => (220, 220, 220),
        SystemColor::Buttontext
        | SystemColor::Canvastext
        | SystemColor::Captiontext
        | SystemColor::Fieldtext
        | SystemColor::Highlighttext
        | SystemColor::Infotext
        | SystemColor::Marktext
        | SystemColor::Menutext
        | SystemColor::Windowtext => (0, 0, 0),
        SystemColor::Canvas
        | SystemColor::Activecaption
        | SystemColor::Appworkspace
        | SystemColor::Background
        | SystemColor::Field
        | SystemColor::Inactivecaption
        | SystemColor::Infobackground
        | SystemColor::Menu
        | SystemColor::Scrollbar
        | SystemColor::Window => (255, 255, 255),
        SystemColor::Graytext | SystemColor::Inactivecaptiontext => (109, 109, 109),
        SystemColor::Highlight => (0, 65, 198),
        SystemColor::Mark => (255, 235, 59),
    };
    AbsoluteColor::srgb_legacy(red, green, blue, 1.0)
}

fn absolute_srgb_bytes(color: AbsoluteColor) -> [u8; 4] {
    color
        .to_color_space(ColorSpace::Srgb)
        .raw_components()
        .map(|component| (component * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use style_traits::ToCss;

    #[test]
    fn color_mix_preserves_zero_weights_and_serializes_omitted_percentages() {
        // CSS Color 5 no longer rejects a zero-sum mix. The frozen WPT
        // snapshot predates this rule and the percentage serialization update.
        for value in [
            "color-mix(in srgb, red 0%, blue 0%)",
            "color-mix(in srgb, red 0%)",
            "color-mix(in srgb, red 0%, green 0%, blue 0%)",
        ] {
            assert!(parse_specified_color(value).is_some(), "{value}");
            assert_eq!(parse_css_color_to_srgb_bytes(value).unwrap()[3], 0);
        }
        assert!(parse_specified_color("color-mix(in srgb, red -1%, blue)").is_none());
        assert!(parse_specified_color("color-mix(in srgb, red 101%, blue)").is_none());
        assert_eq!(
            parse_specified_color("color-mix(contrast-color(blue) 100%, purple)")
                .unwrap()
                .to_css_string(),
            "color-mix(contrast-color(blue) 100%, purple 0%)",
        );
    }

    #[test]
    fn canvas_colors_preserve_alpha_and_color_space() {
        for (value, expected) in [
            ("red", "#ff0000"),
            ("rgb(1, 2, 3)", "#010203"),
            ("hsl(120deg 100% 50%)", "#00ff00"),
            ("transparent", "rgba(0, 0, 0, 0)"),
            ("rgba(255, 0, 0, 0.5)", "rgba(255, 0, 0, 0.5)"),
            ("color(display-p3 1 0 0)", "color(display-p3 1 0 0)"),
        ] {
            assert_eq!(
                parse_css_color_for_canvas(value, None)
                    .map(|color| color.serialize())
                    .as_deref(),
                Some(expected)
            );
        }
        assert_eq!(
            parse_css_color_for_canvas("currentcolor", Some("rgb(1, 2, 3)"))
                .map(|color| color.serialize())
                .as_deref(),
            Some("#010203")
        );
        assert_eq!(
            parse_css_color_for_canvas("currentcolor", None)
                .map(|color| color.serialize())
                .as_deref(),
            Some("#000000")
        );
        for invalid in [
            "",
            "red green",
            "rgb(1,2,3,0.5,1)",
            "rgba(1,2,3,NaN)",
            "#ggg",
            "inherit",
            "var(--color)",
        ] {
            assert_eq!(parse_css_color_for_canvas(invalid, None), None, "{invalid}");
        }
    }

    #[test]
    fn css_colors_convert_to_opaque_srgb_hex() {
        for (value, expected) in [
            ("#fff", "#ffffff"),
            ("crimson", "#dc143c"),
            ("rgb(1,1,1,0.2)", "#010101"),
            ("hsl(150deg 100 53.5)", "#12ff88"),
            ("color(display-p3 .5 0 0)", "#8c0000"),
            ("color(display-p3 1 0 0)", "#ff0000"),
            ("transparent", "#000000"),
            ("currentColor", "#000000"),
            ("ActiveBorder", "#a9a9a9"),
        ] {
            assert_eq!(
                parse_css_color_to_opaque_srgb_hex(value).as_deref(),
                Some(expected),
                "{value}"
            );
        }
        assert_eq!(parse_css_color_to_opaque_srgb_hex("inherit"), None);
        assert_eq!(parse_css_color_to_opaque_srgb_hex("not-a-color"), None);
    }

    #[test]
    fn css_colors_preserve_alpha_in_srgb_bytes() {
        assert_eq!(
            parse_css_color_to_srgb_bytes("#11223380"),
            Some([0x11, 0x22, 0x33, 0x80])
        );
        assert_eq!(
            parse_css_color_to_srgb_bytes("rgba(255, 0, 0, 0.25)"),
            Some([255, 0, 0, 64])
        );
        assert_eq!(parse_css_color_to_srgb_bytes("not-a-color"), None);
    }

    #[test]
    fn system_color_lookup_rejects_non_system_colors() {
        assert_eq!(css_system_color_srgb("MenuText"), Some((0, 0, 0)));
        assert_eq!(css_system_color_srgb("ActiveBorder"), Some((169, 169, 169)));
        assert_eq!(css_system_color_srgb("red"), None);
    }

    #[test]
    fn color_scheme_flags_use_stylo_grammar() {
        assert_eq!(
            parse_css_color_scheme_flags("normal"),
            Some(ColorSchemeFlags::empty())
        );
        assert_eq!(
            parse_css_color_scheme_flags("light dark"),
            Some(ColorSchemeFlags::LIGHT | ColorSchemeFlags::DARK)
        );
        assert_eq!(
            parse_css_color_scheme_flags("BLUE light"),
            Some(ColorSchemeFlags::LIGHT)
        );
        assert_eq!(
            parse_css_color_scheme_flags("only light"),
            Some(ColorSchemeFlags::ONLY | ColorSchemeFlags::LIGHT)
        );
        for invalid in ["", "light,dark", "light, dark", "only"] {
            assert_eq!(parse_css_color_scheme_flags(invalid), None, "{invalid}");
        }
    }
    #[test]
    fn absolute_canvas_color_retains_out_of_gamut_channels_and_alpha() {
        let color = super::parse_css_color_to_absolute("color(display-p3 1 0 0 / 0.5)")
            .unwrap()
            .to_color_space(style::color::ColorSpace::Srgb);
        assert!(color.components.0 > 1.0);
        assert!(color.components.1 < 0.0);
        assert!(color.components.2 < 0.0);
        assert_eq!(color.alpha, 0.5);
        let p3 = color.to_color_space(style::color::ColorSpace::DisplayP3);
        assert!((p3.components.0 - 1.0).abs() < 0.00001);
        assert!(p3.components.1.abs() < 0.00001);
        assert!(p3.components.2.abs() < 0.00001);
        let color = parse_css_color_for_canvas("color(srgb 1 0 0 / 0.123456)", None).unwrap();
        assert!((color.absolute.alpha - 0.123456).abs() < 0.0000001);
    }
}
