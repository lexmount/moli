use euclid::{Scale, Size2D};
use style::context::QuirksMode;

pub(crate) use moli_css_parse::{
    append_media_query_list_medium, delete_media_query_list_medium, media_query_list_items,
    normalize_media_query_list, parse_media_query_list_with_context,
};

use super::{StyleViewport, StyloStyleEnvironment};
use crate::style_engine::world_key::{DEFAULT_VIEWPORT_HEIGHT, DEFAULT_VIEWPORT_WIDTH};

pub(crate) fn evaluate_media_query_list(
    media_text: &str,
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
    viewport: StyleViewport,
) -> bool {
    evaluate_media_query_list_with_context(
        media_text,
        emulated_media,
        viewport,
        &url::Url::parse("about:blank").expect("static URL"),
        QuirksMode::NoQuirks,
    )
}

pub(crate) fn evaluate_media_query_list_with_context(
    media_text: &str,
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
    viewport: StyleViewport,
    base_url: &url::Url,
    quirks_mode: QuirksMode,
) -> bool {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    if media_text.is_empty() {
        return true;
    }

    let device = media_query_device(emulated_media, viewport, quirks_mode);
    moli_css_parse::evaluate_media_query_list(media_text, &device, base_url, quirks_mode)
}

fn media_query_device(
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
    viewport: StyleViewport,
    quirks_mode: QuirksMode,
) -> style::device::Device {
    let environment = emulated_media
        .map(StyloStyleEnvironment::from_emulated_media)
        .unwrap_or_default();
    let width = viewport.width.unwrap_or(DEFAULT_VIEWPORT_WIDTH as f64) as f32;
    let height = viewport.height.unwrap_or(DEFAULT_VIEWPORT_HEIGHT as f64) as f32;
    let ratio = viewport.device_pixel_ratio.unwrap_or(1.0) as f32;
    moli_css_parse::create_media_query_device(
        environment.stylo_media_type(),
        quirks_mode,
        Size2D::new(width, height),
        Size2D::new(
            viewport.screen_width.unwrap_or(f64::from(width)) as f32 * ratio,
            viewport.screen_height.unwrap_or(f64::from(height)) as f32 * ratio,
        ),
        Scale::new(ratio),
        environment.stylo_prefers_color_scheme(),
        environment.stylo_media_feature_preferences(),
    )
}

/// Responsive source lengths use the shared CSS grammar and initial metrics.
pub(crate) fn evaluate_source_size_list(
    sizes: &str,
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
    viewport: StyleViewport,
) -> f64 {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let url_data =
        style::stylesheets::UrlExtraData::from(url::Url::parse("about:blank").expect("static URL"));
    let context = style::parser::ParserContext::new(
        style::stylesheets::Origin::Author,
        &url_data,
        Some(style::stylesheets::CssRuleType::Media),
        style_traits::ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Default::default(),
        None,
        None,
        Default::default(),
    );
    super::source_sizes::evaluate(
        sizes,
        &context,
        &media_query_device(emulated_media, viewport, QuirksMode::NoQuirks),
    )
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
