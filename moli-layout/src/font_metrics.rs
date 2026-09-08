//! CSS font-relative units read unshaped metrics from the same selected faces
//! as text layout. In particular, `ch`/`ic` select a font covering the requested
//! character; `ex`/`cap` use the primary face's metrics, not a shaped fallback run.

use std::borrow::Cow;

use parley::{
    FontFamily, FontFamilyName, FontWeight, TextStyle,
    fontique::{Attributes, FallbackKey, QueryFamily, QueryFont, QueryStatus, Script},
};
use skrifa::{
    FontRef, MetadataProvider, Tag,
    instance::{Location, LocationRef, Size},
    metrics::GlyphMetrics,
    raw::{TableProvider, types::GlyphId},
};
use style::{
    device::servo::FontMetricsProvider,
    font_metrics::FontMetrics,
    properties::style_structs::Font,
    values::computed::{
        CSSPixelLength, Length,
        font::{GenericFontFamily, QueryFontMetricsFlags},
    },
};

use crate::{
    DocumentFontServices,
    stylo_to_parley::{self, TextBrush},
    text::ParleyFontContext,
};

#[derive(Clone, Copy)]
enum MetricFont {
    Primary,
    Zero,
    Water,
}

impl MetricFont {
    fn character(self) -> char {
        match self {
            Self::Primary => ' ',
            Self::Zero => '0',
            Self::Water => '水',
        }
    }

    fn fallback_script(self) -> Script {
        Script::from_bytes(match self {
            Self::Water => *b"Hani",
            Self::Primary | Self::Zero => *b"Latn",
        })
    }
}

impl FontMetricsProvider for DocumentFontServices {
    fn query_font_metrics(
        &self,
        vertical: bool,
        font: &Font,
        base_size: CSSPixelLength,
        flags: QueryFontMetricsFlags,
    ) -> FontMetrics {
        let mut context = if flags.contains(QueryFontMetricsFlags::USE_USER_FONT_SET) {
            self.context()
        } else {
            self.platform_context()
        };
        let style = TextStyle {
            font_family: FontFamily::List(Cow::Owned(
                font.font_family
                    .families
                    .iter()
                    .map(stylo_to_parley::font_family_name)
                    .collect(),
            )),
            font_size: base_size.px(),
            font_width: stylo_to_parley::font_width(font.font_stretch),
            font_style: stylo_to_parley::font_style(font.font_style),
            font_weight: FontWeight::new(font.font_weight.value()),
            ..TextStyle::default()
        };
        let variations = stylo_to_parley::font_variations(&font.font_variation_settings);
        let mut result = FontMetrics::default();
        if let Some(primary) = select_font(&mut context, style.clone(), MetricFont::Primary)
            && let Ok(face) = FontRef::from_index(primary.blob.as_ref(), primary.index)
        {
            let location = font_location(&face, &primary, &variations);
            let size = Size::new(base_size.px());
            let metrics = face.metrics(size, &location);
            let glyphs = face.glyph_metrics(size, &location);
            let height = |reported: Option<f32>, character| {
                reported
                    .filter(|value| *value > 0.0)
                    .or_else(|| {
                        glyphs
                            .bounds(GlyphId::new(primary.charmap()?.map(character)?))
                            .map(|bounds| bounds.y_max)
                            .filter(|value| *value > 0.0)
                    })
                    .map(CSSPixelLength::new)
            };
            result.ascent = CSSPixelLength::new(metrics.ascent);
            result.x_height = height(metrics.x_height, 'x');
            result.cap_height = height(metrics.cap_height, 'H');
        }
        if flags.contains(QueryFontMetricsFlags::NEEDS_CH) {
            result.zero_advance_measure = advance(
                &mut context,
                style.clone(),
                MetricFont::Zero,
                vertical,
                &variations,
            )
            .map(CSSPixelLength::new);
        }
        if flags.contains(QueryFontMetricsFlags::NEEDS_IC) {
            result.ic_width = advance(
                &mut context,
                style,
                MetricFont::Water,
                vertical,
                &variations,
            )
            .map(CSSPixelLength::new);
        }
        result
    }

    fn base_size_for_generic(&self, _generic: GenericFontFamily) -> Length {
        Length::new(16.0)
    }
}

fn select_font(
    context: &mut ParleyFontContext,
    mut style: TextStyle<'static, 'static, TextBrush>,
    request: MetricFont,
) -> Option<QueryFont> {
    // The first available CSS font must include space in its unicode-range,
    // even when its x-height/cap-height come directly from font-wide metrics.
    context.resolve_font_families(&mut style, Some(request.character()));
    let FontFamily::List(families) = &style.font_family else {
        unreachable!("CSS unit queries supply a parsed family list")
    };
    let font_context = &mut context.font_context;
    let mut query = font_context
        .collection
        .query(&mut font_context.source_cache);
    query.set_families(families.iter().map(|family| match family {
        FontFamilyName::Named(name) => QueryFamily::Named(name),
        FontFamilyName::Generic(family) => QueryFamily::Generic(*family),
    }));
    query.set_attributes(Attributes::new(
        style.font_width,
        style.font_style,
        style.font_weight,
    ));
    query.set_fallbacks(FallbackKey::new(request.fallback_script(), None));
    let mut selected = None;
    query.matches_with(|font| {
        if !matches!(request, MetricFont::Primary)
            && font
                .charmap()
                .and_then(|map| map.map(request.character()))
                .is_none()
        {
            return QueryStatus::Continue;
        }
        selected = Some(font.clone());
        QueryStatus::Stop
    });
    selected
}

fn font_location(
    face: &FontRef<'_>,
    font: &QueryFont,
    variations: &[parley::FontVariation],
) -> Location {
    // Fontique maps CSS width/style/weight to variable axes. As in Parley's
    // shaper, explicit font-variation-settings override those mapped values.
    face.axes().location(
        font.synthesis.variation_settings().iter().copied().chain(
            variations
                .iter()
                .map(|v| (Tag::from_be_bytes(v.tag.to_bytes()), v.value)),
        ),
    )
}

fn advance(
    context: &mut ParleyFontContext,
    style: TextStyle<'static, 'static, TextBrush>,
    request: MetricFont,
    vertical: bool,
    variations: &[parley::FontVariation],
) -> Option<f32> {
    let size = style.font_size;
    let font = select_font(context, style, request)?;
    let face = FontRef::from_index(font.blob.as_ref(), font.index).ok()?;
    let glyph = GlyphId::new(font.charmap()?.map(request.character())?);
    let location = font_location(&face, &font, variations);
    if vertical {
        vertical_advance(&face, glyph, size, (&location).into())
    } else {
        GlyphMetrics::new(&face, Size::new(size), &location).advance_width(glyph)
    }
}

fn vertical_advance(
    face: &FontRef<'_>,
    glyph: GlyphId,
    size: f32,
    location: LocationRef<'_>,
) -> Option<f32> {
    let Ok(vmtx) = face.vmtx() else {
        // Fonts without vertical metrics use an em advance for upright text.
        return Some(size);
    };
    let mut units = f32::from(vmtx.advance(glyph)?);
    let coords = location.coords();
    if let Ok(vvar) = face.vvar() {
        units += vvar.advance_height_delta(glyph, coords).ok()?.to_f32();
    } else if !coords.is_empty()
        && let (Ok(gvar), Ok(glyf), Ok(loca)) = (face.gvar(), face.glyf(), face.loca(None))
        && let Some(deltas) = gvar
            .phantom_point_deltas(&glyf, &loca, coords, glyph)
            .ok()?
    {
        units += (deltas[2].y - deltas[3].y).to_f32();
    }
    let units_per_em = face.head().ok()?.units_per_em();
    (units_per_em != 0).then(|| units * size / f32::from(units_per_em))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DocumentLayoutServices, SystemFontPolicy, WebFontFace, WebFontRegistration,
        WebFontUnicodeRange,
    };
    use style::values::{
        computed::font::{FamilyName, FontFamilyNameSyntax, SingleFontFamily},
        generics::font::{FontTag, VariationValue},
    };

    const VARIABLE: &[u8] = include_bytes!("../tests/fixtures/moli-metrics-variable.ttf");
    const LATIN: &[u8] = include_bytes!("../tests/fixtures/moli-ahem.ttf");
    const FLAGS: QueryFontMetricsFlags = QueryFontMetricsFlags::USE_USER_FONT_SET
        .union(QueryFontMetricsFlags::NEEDS_CH)
        .union(QueryFontMetricsFlags::NEEDS_IC);

    fn font(families: &[&str], weight: f32) -> Font {
        let mut font = Font::initial_values();
        font.font_family.families.list = style::ArcSlice::from_iter(families.iter().map(|name| {
            SingleFontFamily::FamilyName(FamilyName {
                name: (*name).into(),
                syntax: FontFamilyNameSyntax::Quoted,
            })
        }));
        font.font_weight = style::values::computed::font::FontWeight::from_float(weight);
        font
    }

    fn fixed_fonts() -> DocumentFontServices {
        DocumentFontServices::with_system_font_policy(SystemFontPolicy::Disabled)
    }

    fn register(fonts: &DocumentFontServices, slot: &str, face: WebFontFace, bytes: &[u8]) {
        fonts
            .register_web_font(WebFontRegistration::new(slot, face, bytes.to_vec()))
            .unwrap();
    }

    fn assert_length(actual: Option<CSSPixelLength>, expected: f32) {
        let actual = actual
            .expect("metric must come from the selected font")
            .px();
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn font_units_use_variable_horizontal_vertical_and_height_metrics() {
        let fonts = fixed_fonts();
        register(&fonts, "variable", WebFontFace::new("Metrics"), VARIABLE);
        for size in [20.0, 40.0] {
            for (weight, zero, water, vertical_zero, x, cap) in [
                (100.0, 0.0, 0.8, 0.4, 0.3, 0.55),
                (400.0, 0.5, 1.0, 0.7, 0.4, 0.65),
                (700.0, 0.75, 1.2, 0.9, 0.5, 0.75),
            ] {
                let font = font(&["Metrics"], weight);
                let horizontal =
                    fonts.query_font_metrics(false, &font, CSSPixelLength::new(size), FLAGS);
                assert_length(horizontal.zero_advance_measure, zero * size);
                assert_length(horizontal.ic_width, water * size);
                assert_length(horizontal.x_height, x * size);
                assert_length(horizontal.cap_height, cap * size);
                let vertical =
                    fonts.query_font_metrics(true, &font, CSSPixelLength::new(size), FLAGS);
                assert_length(vertical.zero_advance_measure, vertical_zero * size);
                assert_length(vertical.ic_width, (vertical_zero + 0.5) * size);
            }
        }
    }

    #[test]
    fn explicit_variation_overrides_css_weight_without_shaping() {
        let fonts = fixed_fonts();
        register(&fonts, "variable", WebFontFace::new("Metrics"), VARIABLE);
        let services = DocumentLayoutServices::with_fonts(fonts.clone());
        let mut font = font(&["Metrics"], 700.0);
        font.font_variation_settings.0 = [VariationValue {
            tag: FontTag(u32::from_be_bytes(*b"wght")),
            value: 100.0,
        }]
        .into_iter()
        .collect();
        let metrics = fonts.query_font_metrics(false, &font, CSSPixelLength::new(20.0), FLAGS);
        assert_length(metrics.zero_advance_measure, 0.0);
        assert_length(metrics.x_height, 6.0);
        assert!(
            !services.is_initialized(),
            "style queries must not create layout scratch space"
        );
    }

    #[test]
    fn metric_glyphs_follow_capability_then_range_and_family_fallback() {
        let fonts = fixed_fonts();
        register(
            &fonts,
            "latin",
            WebFontFace::new("Segmented").with_unicode_ranges([
                WebFontUnicodeRange::new(0x20, 0x20),
                WebFontUnicodeRange::new(0x41, 0x7f),
            ]),
            LATIN,
        );
        register(
            &fonts,
            "metrics",
            WebFontFace::new("Segmented").with_unicode_ranges([
                WebFontUnicodeRange::new(0x30, 0x30),
                WebFontUnicodeRange::new(0x6c34, 0x6c34),
            ]),
            VARIABLE,
        );
        register(
            &fonts,
            "bold",
            WebFontFace::new("Segmented")
                .with_weight(700.0)
                .with_unicode_ranges([WebFontUnicodeRange::new(0x41, 0x7f)]),
            LATIN,
        );
        register(&fonts, "fallback", WebFontFace::new("Fallback"), LATIN);
        let regular = fonts.query_font_metrics(
            false,
            &font(&["Segmented"], 400.0),
            CSSPixelLength::new(20.0),
            FLAGS,
        );
        assert_length(regular.zero_advance_measure, 10.0);
        assert_length(regular.ic_width, 20.0);
        assert_length(regular.x_height, 16.0);
        let bold = fonts.query_font_metrics(
            false,
            &font(&["Segmented", "Fallback"], 700.0),
            CSSPixelLength::new(20.0),
            FLAGS,
        );
        assert_length(bold.zero_advance_measure, 12.0);
        assert!(
            bold.ic_width.is_none(),
            "a different capability group must not supply the missing water ideograph"
        );
    }

    #[test]
    fn font_publication_and_removal_reach_existing_style_and_layout_consumers() {
        let fonts = fixed_fonts();
        let mut services = DocumentLayoutServices::with_fonts(fonts.clone());
        let font = font(&["Ahem"], 400.0);
        let query = || fonts.query_font_metrics(false, &font, CSSPixelLength::new(20.0), FLAGS);
        assert!(
            query().zero_advance_measure.is_none(),
            "an unloaded family must not have invented Ahem metrics"
        );
        services
            .register_web_font(WebFontRegistration::new(
                "face",
                WebFontFace::new("Ahem"),
                VARIABLE.to_vec(),
            ))
            .unwrap();
        assert_length(query().zero_advance_measure, 10.0);
        services
            .register_web_font(WebFontRegistration::new(
                "face",
                WebFontFace::new("Ahem"),
                LATIN.to_vec(),
            ))
            .unwrap();
        assert_length(query().zero_advance_measure, 12.0);
        assert!(services.remove_web_font("face"));
        assert!(query().zero_advance_measure.is_none());
        register(&fonts, "face", WebFontFace::new("Ahem"), LATIN);
        fonts.clear_web_fonts();
        assert_eq!(services.web_font_count(), 0);
        assert!(query().zero_advance_measure.is_none());
    }

    #[test]
    fn media_font_metrics_never_consult_downloaded_faces() {
        let fonts = fixed_fonts();
        register(&fonts, "face", WebFontFace::new("Metrics"), VARIABLE);
        let font = font(&["Metrics"], 400.0);
        let media = fonts.query_font_metrics(
            false,
            &font,
            CSSPixelLength::new(20.0),
            QueryFontMetricsFlags::NEEDS_CH,
        );
        assert!(media.zero_advance_measure.is_none());
        assert_length(
            fonts
                .query_font_metrics(false, &font, CSSPixelLength::new(20.0), FLAGS)
                .zero_advance_measure,
            10.0,
        );
    }

    #[test]
    fn primary_metrics_skip_a_face_whose_range_excludes_space() {
        let fonts = fixed_fonts();
        register(
            &fonts,
            "no-space",
            WebFontFace::new("NoSpace").with_unicode_ranges([WebFontUnicodeRange::new(0x21, 0x7f)]),
            VARIABLE,
        );
        register(&fonts, "fallback", WebFontFace::new("Fallback"), LATIN);
        let metrics = fonts.query_font_metrics(
            false,
            &font(&["NoSpace", "Fallback"], 400.0),
            CSSPixelLength::new(20.0),
            FLAGS,
        );
        assert_length(metrics.x_height, 16.0);
        assert_length(metrics.cap_height, 16.0);
        assert_length(metrics.zero_advance_measure, 10.0);
    }
}
