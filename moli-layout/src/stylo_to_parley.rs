// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The font/style conversion is adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8:
// packages/blitz-dom/src/stylo_to_parley.rs.

use std::{borrow::Cow, sync::Arc};

use parley::setting::Tag;
use parley::{
    FontFamily, FontFamilyName, FontFeature, FontFeatures, FontStyle, FontVariation,
    FontVariations, FontWeight, FontWidth, GenericFamily, LineHeight, OverflowWrap, TextStyle,
    TextWrapMode, WordBreak,
};
use style::{
    computed_values::text_decoration_style::T as StyloTextDecorationStyle,
    properties::ComputedValues,
    values::{
        computed::font::{
            FontFeatureSettings, FontStretch, FontStyle as StyloFontStyle, FontSynthesis,
            FontVariationSettings, GenericFontFamily, SingleFontFamily,
        },
        computed::{
            Length, OverflowWrap as StyloOverflowWrap, TextDecorationLine,
            WordBreak as StyloWordBreak,
        },
        generics::{length::GenericLengthPercentageOrAuto, text::GenericTextDecorationLength},
    },
};

use crate::{PaintColor, PaintPoint, PaintTextDecorationStyle, style::absolute_paint_color};

// Blink only requests synthetic bold for CSS weights at or above 600. Fontique
// reports any requested weight above the selected face as embolden-able, which
// also includes the CSS 500 -> regular 400 match unless we preserve this
// browser-level threshold at the style bridge.
const SYNTHETIC_BOLD_THRESHOLD: f32 = 600.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TextDecorationBrush {
    pub(crate) underline: bool,
    pub(crate) overline: bool,
    pub(crate) line_through: bool,
    pub(crate) style: PaintTextDecorationStyle,
    pub(crate) color: PaintColor,
    pub(crate) thickness: Option<f32>,
    pub(crate) underline_offset: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextShadowBrush {
    pub(crate) color: PaintColor,
    pub(crate) offset: PaintPoint,
    pub(crate) blur_radius: f32,
}

/// Paint data that must travel with a shaped style run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextBrush {
    pub(crate) color: PaintColor,
    /// Whether shaped glyphs from this run are observable paint content.
    ///
    /// Unicode bidi controls and synthetic line-break opportunities still
    /// participate in shaping and line breaking, but never become glyph
    /// fragments in the immutable paint snapshot.
    pub(crate) paint: bool,
    /// Whether this style run is eligible for synthetic bold.
    pub(crate) synthetic_bold: bool,
    pub(crate) decoration: TextDecorationBrush,
    pub(crate) shadows: Arc<[TextShadowBrush]>,
}

impl Default for TextBrush {
    fn default() -> Self {
        Self {
            color: PaintColor::default(),
            paint: true,
            synthetic_bold: false,
            decoration: TextDecorationBrush::default(),
            shadows: Arc::from([]),
        }
    }
}

pub(crate) fn text_style(computed: &ComputedValues) -> TextStyle<'static, 'static, TextBrush> {
    let font = computed.get_font();
    let inherited_text = computed.get_inherited_text();
    let font_size = font.font_size.used_size.0.px();
    let line_height = match font.line_height {
        style::values::computed::font::LineHeight::Normal => LineHeight::MetricsRelative(1.0),
        style::values::computed::font::LineHeight::Number(number) => {
            LineHeight::FontSizeRelative(number.0)
        }
        style::values::computed::font::LineHeight::Length(length) => {
            LineHeight::Absolute(length.0.px())
        }
    };
    let letter_spacing = inherited_text
        .letter_spacing
        .0
        .resolve(Length::new(font_size))
        .px();
    let word_spacing = inherited_text
        .word_spacing
        .resolve(Length::new(font_size))
        .px();

    let families = font
        .font_family
        .families
        .list
        .iter()
        .map(font_family_name)
        .collect::<Vec<_>>();
    let variations = font_variations(&font.font_variation_settings);
    let features = font_features(font);
    let decoration_line = computed.clone_text_decoration_line();
    let current_color = computed.clone_color();
    let fill_color = absolute_paint_color(
        computed
            .clone__webkit_text_fill_color()
            .resolve_to_absolute(&current_color),
    );
    let decoration_color = absolute_paint_color(
        computed
            .clone_text_decoration_color()
            .resolve_to_absolute(&current_color),
    );
    let decoration_style = match computed.clone_text_decoration_style() {
        StyloTextDecorationStyle::Solid | StyloTextDecorationStyle::MozNone => {
            PaintTextDecorationStyle::Solid
        }
        StyloTextDecorationStyle::Double => PaintTextDecorationStyle::Double,
        StyloTextDecorationStyle::Dotted => PaintTextDecorationStyle::Dotted,
        StyloTextDecorationStyle::Dashed => PaintTextDecorationStyle::Dashed,
        StyloTextDecorationStyle::Wavy => PaintTextDecorationStyle::Wavy,
    };
    let decoration_thickness = match computed.clone_text_decoration_thickness() {
        GenericTextDecorationLength::LengthPercentage(length) => {
            Some(length.resolve(Length::new(font_size)).px().max(0.0))
        }
        GenericTextDecorationLength::Auto | GenericTextDecorationLength::FromFont => None,
    };
    let underline_offset = match computed.clone_text_underline_offset() {
        GenericLengthPercentageOrAuto::LengthPercentage(length) => {
            Some(length.resolve(Length::new(font_size)).px())
        }
        GenericLengthPercentageOrAuto::Auto => None,
    };
    let shadows = computed
        .clone_text_shadow()
        .0
        .iter()
        .map(|shadow| TextShadowBrush {
            color: absolute_paint_color(shadow.color.resolve_to_absolute(&current_color)),
            offset: PaintPoint::new(shadow.horizontal.px(), shadow.vertical.px()),
            blur_radius: shadow.blur.0.px().max(0.0),
        })
        .collect::<Vec<_>>();
    let decoration = TextDecorationBrush {
        underline: decoration_line.contains(TextDecorationLine::UNDERLINE),
        overline: decoration_line.contains(TextDecorationLine::OVERLINE),
        line_through: decoration_line.contains(TextDecorationLine::LINE_THROUGH),
        style: decoration_style,
        color: decoration_color,
        thickness: decoration_thickness,
        underline_offset,
    };

    TextStyle {
        font_family: FontFamily::List(Cow::Owned(families)),
        font_size,
        font_width: font_width(font.font_stretch),
        font_style: font_style(font.font_style),
        font_weight: FontWeight::new(font.font_weight.value()),
        font_variations: FontVariations::List(Cow::Owned(variations)),
        font_features: FontFeatures::List(Cow::Owned(features)),
        locale: None,
        line_height,
        word_spacing,
        letter_spacing,
        text_wrap_mode: match inherited_text.text_wrap_mode {
            style::computed_values::text_wrap_mode::T::Wrap => TextWrapMode::Wrap,
            style::computed_values::text_wrap_mode::T::Nowrap => TextWrapMode::NoWrap,
        },
        overflow_wrap: match inherited_text.overflow_wrap {
            StyloOverflowWrap::Normal => OverflowWrap::Normal,
            StyloOverflowWrap::BreakWord => OverflowWrap::BreakWord,
            StyloOverflowWrap::Anywhere => OverflowWrap::Anywhere,
        },
        word_break: match inherited_text.word_break {
            StyloWordBreak::Normal => WordBreak::Normal,
            StyloWordBreak::BreakAll => WordBreak::BreakAll,
            StyloWordBreak::KeepAll => WordBreak::KeepAll,
        },
        brush: TextBrush {
            color: fill_color,
            paint: true,
            synthetic_bold: font.font_synthesis_weight == FontSynthesis::Auto
                && font.font_weight.value() >= SYNTHETIC_BOLD_THRESHOLD,
            decoration,
            shadows: shadows.into(),
        },
        has_underline: decoration.underline,
        underline_offset,
        underline_size: decoration_thickness,
        underline_brush: None,
        has_strikethrough: decoration.line_through,
        strikethrough_offset: None,
        strikethrough_size: decoration_thickness,
        strikethrough_brush: None,
    }
}

fn font_family_name(family: &SingleFontFamily) -> FontFamilyName<'static> {
    match family {
        SingleFontFamily::FamilyName(name) => {
            let name = name.name.as_ref();
            #[cfg(target_vendor = "apple")]
            if name == "-apple-system" {
                return FontFamilyName::Generic(GenericFamily::SystemUi);
            }
            #[cfg(target_os = "macos")]
            if name == "BlinkMacSystemFont" {
                return FontFamilyName::Generic(GenericFamily::SystemUi);
            }
            FontFamilyName::Named(Cow::Owned(name.to_owned()))
        }
        SingleFontFamily::Generic(generic) => {
            FontFamilyName::Generic(generic_font_family(*generic))
        }
    }
}

fn generic_font_family(family: GenericFontFamily) -> GenericFamily {
    match family {
        GenericFontFamily::None | GenericFontFamily::SansSerif => GenericFamily::SansSerif,
        GenericFontFamily::Serif => GenericFamily::Serif,
        GenericFontFamily::Monospace => GenericFamily::Monospace,
        GenericFontFamily::Cursive => GenericFamily::Cursive,
        GenericFontFamily::Fantasy => GenericFamily::Fantasy,
        GenericFontFamily::SystemUi => GenericFamily::SystemUi,
        GenericFontFamily::GenericFangsong
        | GenericFontFamily::GenericKai
        | GenericFontFamily::GenericKhmerMul
        | GenericFontFamily::GenericNastaliq
        | GenericFontFamily::WebkitGenericFangsong
        | GenericFontFamily::WebkitGenericKai
        | GenericFontFamily::WebkitGenericKhmerMul
        | GenericFontFamily::WebkitGenericNastaliq => GenericFamily::Cursive,
    }
}

fn font_width(stretch: FontStretch) -> FontWidth {
    FontWidth::from_percentage(stretch.0.to_float())
}

fn font_style(style: StyloFontStyle) -> FontStyle {
    match style {
        StyloFontStyle::NORMAL => FontStyle::Normal,
        StyloFontStyle::ITALIC => FontStyle::Italic,
        value => FontStyle::Oblique(Some(value.oblique_degrees())),
    }
}

fn font_variations(settings: &FontVariationSettings) -> Vec<FontVariation> {
    settings
        .0
        .iter()
        .map(|variation| FontVariation {
            tag: Tag::from_bytes(variation.tag.0.to_be_bytes()),
            value: variation.value,
        })
        .collect()
}

fn feature(tag: &[u8; 4], value: u16) -> FontFeature {
    FontFeature {
        tag: Tag::from_bytes(*tag),
        value,
    }
}

fn font_feature_settings(settings: &FontFeatureSettings, output: &mut Vec<FontFeature>) {
    output.extend(settings.0.iter().map(|setting| FontFeature {
        tag: Tag::from_bytes(setting.tag.0.to_be_bytes()),
        value: setting.value as u16,
    }));
}

fn font_features(font: &style::properties::style_structs::Font) -> Vec<FontFeature> {
    use style::computed_values::{
        font_variant_caps::T as Caps, font_variant_position::T as Position,
    };
    use style::values::computed::font::{
        FontVariantEastAsian as EastAsian, FontVariantLigatures as Ligatures,
        FontVariantNumeric as Numeric,
    };

    let mut output = Vec::new();
    let ligatures = font.font_variant_ligatures;
    if ligatures.contains(Ligatures::NONE) {
        output.extend([
            feature(b"liga", 0),
            feature(b"clig", 0),
            feature(b"dlig", 0),
            feature(b"hlig", 0),
            feature(b"calt", 0),
        ]);
    } else {
        for (enabled, disabled, tag) in [
            (
                Ligatures::COMMON_LIGATURES,
                Ligatures::NO_COMMON_LIGATURES,
                b"liga",
            ),
            (
                Ligatures::DISCRETIONARY_LIGATURES,
                Ligatures::NO_DISCRETIONARY_LIGATURES,
                b"dlig",
            ),
            (
                Ligatures::HISTORICAL_LIGATURES,
                Ligatures::NO_HISTORICAL_LIGATURES,
                b"hlig",
            ),
            (Ligatures::CONTEXTUAL, Ligatures::NO_CONTEXTUAL, b"calt"),
        ] {
            if ligatures.contains(enabled) {
                output.push(feature(tag, 1));
                if tag == b"liga" {
                    output.push(feature(b"clig", 1));
                }
            } else if ligatures.contains(disabled) {
                output.push(feature(tag, 0));
                if tag == b"liga" {
                    output.push(feature(b"clig", 0));
                }
            }
        }
    }
    if font.font_variant_caps == Caps::SmallCaps {
        output.push(feature(b"smcp", 1));
    }
    match font.font_variant_position {
        Position::Normal => {}
        Position::Sub => output.push(feature(b"subs", 1)),
        Position::Super => output.push(feature(b"sups", 1)),
    }

    let numeric = font.font_variant_numeric;
    for (flag, tag) in [
        (Numeric::LINING_NUMS, b"lnum"),
        (Numeric::OLDSTYLE_NUMS, b"onum"),
        (Numeric::PROPORTIONAL_NUMS, b"pnum"),
        (Numeric::TABULAR_NUMS, b"tnum"),
        (Numeric::DIAGONAL_FRACTIONS, b"frac"),
        (Numeric::STACKED_FRACTIONS, b"afrc"),
        (Numeric::ORDINAL, b"ordn"),
        (Numeric::SLASHED_ZERO, b"zero"),
    ] {
        if numeric.contains(flag) {
            output.push(feature(tag, 1));
        }
    }

    let east_asian = font.font_variant_east_asian;
    for (flag, tag) in [
        (EastAsian::JIS78, b"jp78"),
        (EastAsian::JIS83, b"jp83"),
        (EastAsian::JIS90, b"jp90"),
        (EastAsian::JIS04, b"jp04"),
        (EastAsian::SIMPLIFIED, b"smpl"),
        (EastAsian::TRADITIONAL, b"trad"),
        (EastAsian::FULL_WIDTH, b"fwid"),
        (EastAsian::PROPORTIONAL_WIDTH, b"pwid"),
        (EastAsian::RUBY, b"ruby"),
    ] {
        if east_asian.contains(flag) {
            output.push(feature(tag, 1));
        }
    }

    // CSS Fonts gives the low-level property the final say. Parley applies the
    // last duplicate tag, so keep this append after the high-level variants.
    font_feature_settings(&font.font_feature_settings, &mut output);
    output.reverse();
    output.sort_by_key(|value| value.tag);
    output.dedup_by_key(|value| value.tag);
    output
}
