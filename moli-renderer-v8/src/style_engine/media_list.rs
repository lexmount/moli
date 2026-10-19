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

use super::{StyleViewport, StyloStyleEnvironment};
use crate::style_engine::world_key::{DEFAULT_VIEWPORT_HEIGHT, DEFAULT_VIEWPORT_WIDTH};

pub(crate) fn normalize_media_query_list(media_text: &str) -> String {
    with_stylo_media_context(|context| parse_stylo_media_list(context, media_text).to_css_string())
}

pub(crate) fn parse_media_query_list_with_context(
    media_text: &str,
    base_url: &url::Url,
    quirks_mode: QuirksMode,
) -> MediaList {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
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

pub(crate) fn evaluate_media_query_list(
    media_text: &str,
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
    viewport: StyleViewport,
) -> bool {
    with_stylo_media_context(|context| {
        let media_list = parse_stylo_media_list(context, media_text);
        let device = media_query_device(emulated_media, viewport);
        let mut custom_media = style::stylesheets::CustomMediaEvaluator::none();
        media_list.evaluate(&device, QuirksMode::NoQuirks, &mut custom_media)
    })
}

/// Resolve a responsive-image source size using CSS syntax and initial font
/// metrics. This needs a device, not an element style or a layout pass.
pub(crate) fn evaluate_source_size_list(
    sizes: &str,
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
    viewport: StyleViewport,
) -> f64 {
    with_stylo_media_context(|context| {
        super::source_sizes::evaluate(
            sizes,
            context,
            &media_query_device(emulated_media, viewport),
        )
    })
}

pub(crate) fn media_query_list_items(media_text: &str) -> Vec<String> {
    with_stylo_media_context(|context| {
        parse_stylo_media_list(context, media_text)
            .media_queries
            .iter()
            .map(ToCss::to_css_string)
            .collect()
    })
}

pub(crate) fn append_media_query_list_medium(media_text: &str, medium: &str) -> Option<String> {
    with_stylo_media_context(|context| {
        let mut media_list = parse_stylo_media_list(context, media_text);
        media_list
            .append_medium(context, medium)
            .then(|| media_list.to_css_string())
    })
}

pub(crate) fn delete_media_query_list_medium(media_text: &str, medium: &str) -> Option<String> {
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
    crate::style_engine::ensure_stylo_browser_compat_prefs();
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

fn media_query_device(
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
    viewport: StyleViewport,
) -> Device {
    let environment = emulated_media
        .map(StyloStyleEnvironment::from_emulated_media)
        .unwrap_or_default();
    let initial_style = ComputedValues::initial_values_with_font_override(Font::initial_values());
    let viewport_width = viewport.width.unwrap_or(DEFAULT_VIEWPORT_WIDTH as f64) as f32;
    let viewport_height = viewport.height.unwrap_or(DEFAULT_VIEWPORT_HEIGHT as f64) as f32;
    let screen_width = viewport.screen_width.unwrap_or(viewport_width as f64) as f32;
    let screen_height = viewport.screen_height.unwrap_or(viewport_height as f64) as f32;
    let ratio = viewport.device_pixel_ratio.unwrap_or(1.0) as f32;
    let mut device = Device::new(
        environment.stylo_media_type(),
        QuirksMode::NoQuirks,
        Size2D::<f32, CSSPixel>::new(viewport_width, viewport_height),
        Size2D::<f32, DevicePixel>::new(screen_width * ratio, screen_height * ratio),
        Scale::<f32, CSSPixel, DevicePixel>::new(ratio),
        Box::new(MediaQueryFontMetricsProvider),
        initial_style,
        environment.stylo_prefers_color_scheme(),
        PointerCapabilities::default(),
        PointerCapabilities::default(),
    );
    device.set_media_feature_preferences(environment.stylo_media_feature_preferences());
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
    use super::*;

    #[test]
    fn source_sizes_resolve_css_math_units_comments_and_escapes() {
        let viewport = StyleViewport::new(Some(1_000.0), Some(500.0));
        for (text, expected) in [
            ("calc(100vw - 20px)", 980.0),
            ("min(100px, max(20px, 5vw)), 200px", 50.0),
            ("clamp(20px, 5vw, 100px)", 50.0),
            ("2em", 32.0),
            ("1in", 96.0),
            ("10vmin", 50.0),
            ("/**/ 1\\70 x /**/", 1.0),
            ("calc(1px", 1.0),
        ] {
            assert_eq!(
                evaluate_source_size_list(text, None, viewport),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn source_sizes_follow_media_conditions_and_invalid_component_boundaries() {
        let viewport = StyleViewport::new(Some(1_000.0), Some(500.0));
        for (text, expected) in [
            ("(max-width: 1200px) 25vw, 80vw", 250.0),
            ("(max-width: 800px) 25vw, 80vw", 800.0),
            ("not (unknown-feature) 1px, 2px", 2.0),
            ("(width: 1000px) or (unknown-feature) 1px, 2px", 1.0),
            ("all 1px, 2px", 2.0),
            ("(unknown-feature) 1px, 10%", 1_000.0),
            ("1/**/px, 2px", 2.0),
            ("((),1px", 1_000.0),
            ("var(--size), 2px", 2.0),
        ] {
            assert_eq!(
                evaluate_source_size_list(text, None, viewport),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn source_sizes_use_the_document_viewport_and_emulated_media_features() {
        let viewport = StyleViewport::new(Some(1_000.0), Some(500.0))
            .with_screen_size(Some(1_920.0), Some(1_080.0));
        let dark = crate::protocol_types::EmulatedMediaOverrides {
            color_scheme: Some("dark".into()),
            ..Default::default()
        };
        let sizes = "(prefers-color-scheme: dark) 25vw, 80vw";
        assert_eq!(
            evaluate_source_size_list(sizes, Some(&dark), viewport),
            250.0
        );
        assert_eq!(evaluate_source_size_list(sizes, None, viewport), 800.0);
        assert_eq!(
            evaluate_source_size_list("(device-width: 1920px) 1px, 2px", None, viewport),
            1.0
        );
    }

    #[test]
    fn source_sizes_validate_math_before_property_clamping() {
        let viewport = StyleViewport::new(Some(1_000.0), Some(500.0));
        for (text, expected) in [
            ("calc(1px / 0), 17px", 17.0),
            ("calc(0px / 0), 17px", 17.0),
            ("calc(-1px / 0), 17px", 17.0),
            ("calc(0), 23px", 23.0),
            ("min(-100px, 1px), 17px", 0.0),
        ] {
            assert_eq!(
                evaluate_source_size_list(text, None, viewport),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn source_sizes_media_zero_math_keeps_units_and_unknown_conditions() {
        let viewport = StyleViewport::new(Some(1_000.0), Some(500.0));
        for (text, expected) in [
            ("(min-width: calc(0)) 1px, 2px", 1.0),
            ("(min-width: max(0, -1)) 1px, 2px", 1.0),
            ("(min-width: calc(2 / 2 - 1)) 1px, 2px", 1.0),
            ("(min-width: calc(1px - 1px)) 1px, 2px", 1.0),
            (
                "/*☃*/ (min-width: calc(0)) and (min-height: max(0, -1)) 1px, 2px",
                1.0,
            ),
            ("(min-width: calc(1)) 1px, 2px", 2.0),
            ("(min-width: calc(0 / 0)) 1px, 2px", 2.0),
            ("not (unknown-feature: calc(0)) 1px, 2px", 2.0),
        ] {
            assert_eq!(
                evaluate_source_size_list(text, None, viewport),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn media_query_list_normalization_uses_stylo_serialization() {
        assert_eq!(
            normalize_media_query_list("screen  and  (min-width: 480px),print"),
            "screen and (min-width: 480px), print"
        );
        assert_eq!(normalize_media_query_list("spEech"), "speech");
        assert_eq!(normalize_media_query_list("all and (WiDtH)"), "(width)");
        assert_eq!(normalize_media_query_list("all and (cOLor)"), "(color)");
        assert_eq!(
            normalize_media_query_list("all and (not-a-real-feature)"),
            "(not-a-real-feature)"
        );
        assert_eq!(
            media_query_list_items("screen and (min-width: 480px), print"),
            vec!["screen and (min-width: 480px)", "print"]
        );
    }

    #[test]
    fn media_query_list_mutation_uses_stylo_cssom_methods() {
        assert_eq!(
            append_media_query_list_medium("screen, print", "screen").as_deref(),
            Some("print, screen")
        );
        assert_eq!(
            delete_media_query_list_medium("screen, print", "screen").as_deref(),
            Some("print")
        );
        assert!(delete_media_query_list_medium("screen", "print").is_none());
        assert!(append_media_query_list_medium("screen", "").is_none());
    }

    #[test]
    fn media_query_list_evaluation_uses_stylo_device() {
        assert!(evaluate_media_query_list(
            "(width >= 768px) and (color)",
            None,
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(!evaluate_media_query_list(
            "print and (width >= 768px)",
            None,
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "print and (prefers-color-scheme: dark)",
            Some(&crate::protocol_types::EmulatedMediaOverrides {
                media: Some("print".to_owned()),
                color_scheme: Some("dark".to_owned()),
                ..Default::default()
            }),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "(prefers-color-scheme: dark) and (prefers-reduced-motion: reduce)",
            Some(&crate::protocol_types::EmulatedMediaOverrides {
                color_scheme: Some("dark".to_owned()),
                reduced_motion: Some("reduce".to_owned()),
                ..Default::default()
            }),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(!evaluate_media_query_list(
            "(prefers-reduced-motion) and (prefers-contrast)",
            None,
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "(prefers-reduced-motion: no-preference) and (prefers-contrast: no-preference)",
            None,
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "(prefers-reduced-motion: no-preference) and (forced-colors: active)",
            Some(&crate::protocol_types::EmulatedMediaOverrides {
                reduced_motion: Some("no-preference".to_owned()),
                forced_colors: Some("active".to_owned()),
                ..Default::default()
            }),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "(prefers-reduced-transparency: reduce)",
            Some(&crate::protocol_types::EmulatedMediaOverrides {
                reduced_transparency: Some("reduce".to_owned()),
                ..Default::default()
            }),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
    }

    #[test]
    fn media_query_list_evaluation_keeps_viewport_and_screen_size_separate() {
        let viewport = StyleViewport::new(Some(800.0), Some(600.0))
            .with_screen_size(Some(1920.0), Some(1080.0));

        assert!(evaluate_media_query_list(
            "(width: 800px) and (height: 600px)",
            None,
            viewport,
        ));
        assert!(evaluate_media_query_list(
            "(device-width: 1920px) and (device-height: 1080px)",
            None,
            viewport,
        ));
        assert!(evaluate_media_query_list(
            "(device-aspect-ratio: 16 / 9)",
            None,
            viewport,
        ));
        assert!(!evaluate_media_query_list(
            "(device-width: 800px) or (device-height: 600px)",
            None,
            viewport,
        ));
    }
}
