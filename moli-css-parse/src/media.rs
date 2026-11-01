//! Native Stylo media-query parsing and evaluation, shared by DOM and renderer users.

use cssparser::Parser;
use euclid::{Scale, Size2D};
use std::fmt::Debug;
use style::{
    context::QuirksMode,
    device::{Device, servo::FontMetricsProvider},
    font_metrics::FontMetrics,
    media_queries::MediaList,
    parser::ParserContext,
    properties::{ComputedValues, style_structs::Font},
    servo::media_features::PointerCapabilities,
    stylesheets::{CssRuleType, Origin, UrlExtraData},
    values::{
        computed::{
            CSSPixelLength, Length,
            font::{GenericFontFamily, SingleFontFamily},
        },
        specified::font::QueryFontMetricsFlags,
    },
};
use style_traits::{CSSPixel, DevicePixel, ParsingMode, ToCss};

pub fn normalize_media_query_list(media_text: &str) -> String {
    with_stylo_media_context(|context| parse_stylo_media_list(context, media_text).to_css_string())
}

pub fn parse_media_query_list_with_context(
    media_text: &str,
    base_url: &url::Url,
    quirks_mode: QuirksMode,
) -> MediaList {
    let url_data = UrlExtraData::from(base_url.clone());
    let mut context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Media),
        ParsingMode::DEFAULT,
        quirks_mode,
        Default::default(),
        None,
        None,
        Default::default(),
    );
    parse_stylo_media_list(&mut context, media_text)
}

pub fn evaluate_media_query_list(
    media_text: &str,
    device: &Device,
    base_url: &url::Url,
    quirks_mode: QuirksMode,
) -> bool {
    let media_list = parse_media_query_list_with_context(media_text, base_url, quirks_mode);
    let mut custom_media = style::stylesheets::CustomMediaEvaluator::none();
    media_list.evaluate(device, quirks_mode, &mut custom_media)
}

/// Evaluates an un-emulated screen environment with explicit viewport and
/// screen sizes. Live embedders can supply preferences through their Device.
pub fn screen_media_query_matches(
    media_text: &str,
    quirks_mode: bool,
    viewport: (f32, f32),
    screen: (f32, f32),
    pixel_ratio: f32,
) -> bool {
    let quirks_mode = if quirks_mode {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let device = create_media_query_device(
        style::media_queries::MediaType::screen(),
        quirks_mode,
        Size2D::new(viewport.0, viewport.1),
        Size2D::new(screen.0 * pixel_ratio, screen.1 * pixel_ratio),
        Scale::new(pixel_ratio),
        style::queries::values::PrefersColorScheme::Light,
        Default::default(),
    );
    evaluate_media_query_list(
        media_text,
        &device,
        &url::Url::parse("about:blank").expect("static URL"),
        quirks_mode,
    )
}

pub fn media_query_list_items(media_text: &str) -> Vec<String> {
    with_stylo_media_context(|context| {
        parse_stylo_media_list(context, media_text)
            .media_queries
            .iter()
            .map(ToCss::to_css_string)
            .collect()
    })
}

pub fn append_media_query_list_medium(media_text: &str, medium: &str) -> Option<String> {
    with_stylo_media_context(|context| {
        let mut media_list = parse_stylo_media_list(context, media_text);
        media_list
            .append_medium(context, medium)
            .then(|| media_list.to_css_string())
    })
}

pub fn delete_media_query_list_medium(media_text: &str, medium: &str) -> Option<String> {
    with_stylo_media_context(|context| {
        let mut media_list = parse_stylo_media_list(context, media_text);
        media_list
            .delete_medium(context, medium)
            .then(|| media_list.to_css_string())
    })
}

fn parse_stylo_media_list(context: &mut ParserContext, media_text: &str) -> MediaList {
    let input = media_text;
    let mut parser = Parser::new(input);
    MediaList::parse(context, &mut parser)
}

fn with_stylo_media_context<R>(f: impl FnOnce(&mut ParserContext) -> R) -> R {
    let url_data = UrlExtraData::from(
        url::Url::parse("about:blank").expect("static about:blank URL should parse"),
    );
    let mut context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Media),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Default::default(),
        None,
        None,
        Default::default(),
    );
    f(&mut context)
}

#[derive(Debug)]
struct MediaQueryFontMetricsProvider;

pub fn create_media_query_device(
    media_type: style::media_queries::MediaType,
    quirks_mode: QuirksMode,
    viewport: Size2D<f32, CSSPixel>,
    screen: Size2D<f32, DevicePixel>,
    ratio: Scale<f32, CSSPixel, DevicePixel>,
    color_scheme: style::queries::values::PrefersColorScheme,
    preferences: style::device::servo::ServoMediaFeaturePreferences,
) -> Device {
    let initial_style = ComputedValues::initial_values_with_font_override(Font::initial_values());
    let mut device = Device::new(
        media_type,
        quirks_mode,
        viewport,
        screen,
        ratio,
        Box::new(MediaQueryFontMetricsProvider),
        initial_style,
        color_scheme,
        PointerCapabilities::default(),
        PointerCapabilities::default(),
    );
    device.set_media_feature_preferences(preferences);
    device
}

impl FontMetricsProvider for MediaQueryFontMetricsProvider {
    fn query_font_metrics(
        &self,
        _vertical: bool,
        font: &Font,
        base_size: CSSPixelLength,
        _flags: QueryFontMetricsFlags,
    ) -> FontMetrics {
        let mut metrics = FontMetrics::default();
        if font.clone_font_family().families.iter().next().is_some_and(
            |family| matches!(family, SingleFontFamily::FamilyName(name) if AsRef::<str>::as_ref(&name.name).eq_ignore_ascii_case("Ahem")),
        ) {
            metrics.zero_advance_measure = Some(base_size);
        }
        metrics
    }

    fn base_size_for_generic(&self, _generic: GenericFontFamily) -> Length {
        Length::new(16.0)
    }
}

#[cfg(test)]
mod tests {
    use super::screen_media_query_matches;

    #[test]
    fn media_queries_keep_document_viewport_screen_and_resolution_separate() {
        let matches =
            |media| screen_media_query_matches(media, false, (320.0, 200.0), (1920.0, 1080.0), 2.0);
        for media in [
            "",
            "/* empty */",
            "screen, print",
            "only screen",
            "not print",
            "(width: 320px) and (height: 200px)",
            "(device-width: 1920px) and (device-height: 1080px)",
            "screen and (color) and (resolution: 2dppx)",
            r"\73 creen,print",
        ] {
            assert!(matches(media), "media={media:?}");
        }
        for media in [
            "print",
            "(min-width: 768px)",
            "not screen",
            r#""print,screen""#,
        ] {
            assert!(!matches(media), "media={media:?}");
        }
    }
}
