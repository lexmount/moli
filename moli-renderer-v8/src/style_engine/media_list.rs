use style::context::QuirksMode;

pub(crate) use moli_css_parse::{
    append_media_query_list_medium, delete_media_query_list_medium, media_query_list_items,
    normalize_media_query_list, parse_media_query_list_with_context,
};

use super::stylesheet::new_style_device_with_viewport;
use super::{StyleViewport, StyloStyleEnvironment};

pub(crate) fn evaluate_media_query_list(
    media_text: &str,
    environment: StyloStyleEnvironment,
    viewport: StyleViewport,
) -> bool {
    evaluate_media_query_list_with_context(
        media_text,
        environment,
        viewport,
        &url::Url::parse("about:blank").expect("static URL"),
        QuirksMode::NoQuirks,
    )
}

pub(crate) fn evaluate_media_query_list_with_context(
    media_text: &str,
    environment: StyloStyleEnvironment,
    viewport: StyleViewport,
    base_url: &url::Url,
    quirks_mode: QuirksMode,
) -> bool {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    if media_text.is_empty() {
        return true;
    }

    let device = new_style_device_with_viewport(viewport, environment, quirks_mode);
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
            StyloStyleEnvironment::default(),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(!evaluate_media_query_list(
            "print and (width >= 768px)",
            StyloStyleEnvironment::default(),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "print and (prefers-color-scheme: dark)",
            StyloStyleEnvironment::from_emulated_media(
                &crate::protocol_types::EmulatedMediaOverrides {
                    media: Some("print".to_owned()),
                    color_scheme: Some("dark".to_owned()),
                    ..Default::default()
                }
            ),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "(prefers-color-scheme: dark) and (prefers-reduced-motion: reduce)",
            StyloStyleEnvironment::from_emulated_media(
                &crate::protocol_types::EmulatedMediaOverrides {
                    color_scheme: Some("dark".to_owned()),
                    reduced_motion: Some("reduce".to_owned()),
                    ..Default::default()
                }
            ),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(!evaluate_media_query_list(
            "(prefers-reduced-motion) and (prefers-contrast)",
            StyloStyleEnvironment::default(),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "(prefers-reduced-motion: no-preference) and (prefers-contrast: no-preference)",
            StyloStyleEnvironment::default(),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "(prefers-reduced-motion: no-preference) and (forced-colors: active)",
            StyloStyleEnvironment::from_emulated_media(
                &crate::protocol_types::EmulatedMediaOverrides {
                    reduced_motion: Some("no-preference".to_owned()),
                    forced_colors: Some("active".to_owned()),
                    ..Default::default()
                }
            ),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
        assert!(evaluate_media_query_list(
            "(prefers-reduced-transparency: reduce)",
            StyloStyleEnvironment::from_emulated_media(
                &crate::protocol_types::EmulatedMediaOverrides {
                    reduced_transparency: Some("reduce".to_owned()),
                    ..Default::default()
                }
            ),
            StyleViewport::new(Some(1920.0), Some(1080.0)),
        ));
    }

    #[test]
    fn media_query_list_evaluation_keeps_viewport_and_screen_size_separate() {
        let viewport = StyleViewport::new(Some(800.0), Some(600.0))
            .with_screen_size(Some(1920.0), Some(1080.0));

        assert!(evaluate_media_query_list(
            "(width: 800px) and (height: 600px)",
            StyloStyleEnvironment::default(),
            viewport,
        ));
        assert!(evaluate_media_query_list(
            "(device-width: 1920px) and (device-height: 1080px)",
            StyloStyleEnvironment::default(),
            viewport,
        ));
        assert!(evaluate_media_query_list(
            "(device-aspect-ratio: 16 / 9)",
            StyloStyleEnvironment::default(),
            viewport,
        ));
        assert!(!evaluate_media_query_list(
            "(device-width: 800px) or (device-height: 600px)",
            StyloStyleEnvironment::default(),
            viewport,
        ));
    }
}
