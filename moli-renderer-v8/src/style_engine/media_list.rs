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
    let environment = emulated_media
        .map(StyloStyleEnvironment::from_emulated_media)
        .unwrap_or_default();
    let width = viewport.width.unwrap_or(DEFAULT_VIEWPORT_WIDTH as f64) as f32;
    let height = viewport.height.unwrap_or(DEFAULT_VIEWPORT_HEIGHT as f64) as f32;
    let ratio = viewport.device_pixel_ratio.unwrap_or(1.0) as f32;
    let device = moli_css_parse::create_media_query_device(
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
    );
    moli_css_parse::evaluate_media_query_list(media_text, &device, base_url, quirks_mode)
}

#[cfg(test)]
mod tests {
    use super::*;

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
