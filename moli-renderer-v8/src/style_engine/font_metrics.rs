//! Browser font preferences shared by the cascade and media-query devices.

use style::{
    device::servo::FontMetricsProvider,
    font_metrics::FontMetrics,
    properties::style_structs::Font,
    values::{
        computed::{
            CSSPixelLength, Length,
            font::{GenericFontFamily, SingleFontFamily},
        },
        specified::font::QueryFontMetricsFlags,
    },
};

#[derive(Debug)]
pub(super) struct BrowserFontMetricsProvider;

impl FontMetricsProvider for BrowserFontMetricsProvider {
    fn query_font_metrics(
        &self,
        _vertical: bool,
        font: &Font,
        base_size: CSSPixelLength,
        _flags: QueryFontMetricsFlags,
    ) -> FontMetrics {
        let mut metrics = FontMetrics::default();
        if font.clone_font_family().families.iter().next().is_some_and(
            |family| matches!(family, SingleFontFamily::FamilyName(name) if name.name.as_ref().eq_ignore_ascii_case("Ahem")),
        ) {
            metrics.zero_advance_measure = Some(base_size);
        }
        metrics
    }

    fn base_size_for_generic(&self, generic: GenericFontFamily) -> Length {
        Length::new(if generic == GenericFontFamily::Monospace {
            13.0
        } else {
            16.0
        })
    }

    fn preserve_font_size_keywords_in_relative_values(&self) -> bool {
        false
    }
}
