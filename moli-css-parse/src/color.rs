//! CSS color parsing helpers shared by HTML and CSS observable surfaces.

use cssparser::{Parser, ParserInput};
use style::{
    color::AbsoluteColor,
    context::QuirksMode,
    custom_properties::AttrTaint,
    parser::{Parse, ParserContext},
    stylesheets::{CssRuleType, Origin, UrlExtraData},
    values::specified::color::{Color, ColorScheme, ColorSchemeFlags, SystemColor},
};
use style_traits::ParsingMode;

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
    let color = parse_specified_color(value)?;
    let absolute = match color {
        Color::CurrentColor => AbsoluteColor::BLACK,
        Color::System(system) => system_color_absolute(system),
        color => color
            .to_computed_color(None)
            .ok()?
            .resolve_to_absolute(&AbsoluteColor::BLACK),
    };
    Some(absolute.to_nscolor().to_le_bytes())
}

/// Resolve a CSS system-color keyword through the headless Servo light
/// palette.
pub fn css_system_color_srgb(value: &str) -> Option<(u8, u8, u8)> {
    let Color::System(system) = parse_specified_color(value)? else {
        return None;
    };
    let [red, green, blue, _alpha] = system_color_absolute(system).to_nscolor().to_le_bytes();
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
    let mut input = ParserInput::new(value);
    Parser::new(&mut input)
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
    let mut input = ParserInput::new(value);
    Parser::new(&mut input)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
