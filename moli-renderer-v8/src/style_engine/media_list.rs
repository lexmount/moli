use cssparser::{Parser, ParserInput};
use euclid::{Scale, Size2D};
use style::{
    context::QuirksMode,
    device::Device,
    media_queries::MediaList,
    parser::ParserContext,
    properties::{ComputedValues, style_structs::Font},
    servo::media_features::PointerCapabilities,
    stylesheets::{CssRuleType, Origin, UrlExtraData},
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
    let mut input = ParserInput::new(media_text);
    let mut parser = Parser::new(&mut input);
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
    let mut device = Device::new(
        environment.stylo_media_type(),
        QuirksMode::NoQuirks,
        Size2D::<f32, CSSPixel>::new(viewport_width, viewport_height),
        Size2D::<f32, DevicePixel>::new(screen_width, screen_height),
        Scale::<f32, CSSPixel, DevicePixel>::new(1.0),
        Box::new(moli_layout::DocumentFontServices::default()),
        initial_style,
        environment.stylo_prefers_color_scheme(),
        PointerCapabilities::default(),
        PointerCapabilities::default(),
    );
    device.set_media_feature_preferences(environment.stylo_media_feature_preferences());
    device
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
