use style::{
    device::servo::ServoMediaFeaturePreferences,
    media_queries::MediaType,
    queries::values::PrefersColorScheme,
    servo::media_features::PrefersContrast,
    values::specified::color::{ColorSchemeFlags, ForcedColors},
};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
};

/// Cheap, Document-local identity of the connected ShadowRoot universe
/// sampled for one style observation. The actual root vector is cloned only
/// when this version no longer matches the retained Document world.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StyleTreeScopeVersions {
    document_tree_scopes: u64,
}

impl StyleTreeScopeVersions {
    pub(crate) fn current(host: &DomHost, document: Option<DomHandle>) -> Self {
        Self {
            document_tree_scopes: document
                .map(|document| host.document_tree_scope_version(document))
                .unwrap_or(0),
        }
    }

    #[cfg(test)]
    pub(super) const fn for_test(document_tree_scopes: u64) -> Self {
        Self {
            document_tree_scopes,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StyleViewport {
    pub(crate) width: Option<f64>,
    pub(crate) height: Option<f64>,
    pub(crate) screen_width: Option<f64>,
    pub(crate) screen_height: Option<f64>,
    pub(crate) device_pixel_ratio: Option<f64>,
}

impl StyleViewport {
    pub(crate) const fn new(width: Option<f64>, height: Option<f64>) -> Self {
        Self {
            width,
            height,
            screen_width: None,
            screen_height: None,
            device_pixel_ratio: None,
        }
    }

    pub(crate) const fn from_width(width: Option<f64>) -> Self {
        Self {
            width,
            height: None,
            screen_width: None,
            screen_height: None,
            device_pixel_ratio: None,
        }
    }

    pub(crate) const fn with_screen_size(
        self,
        screen_width: Option<f64>,
        screen_height: Option<f64>,
    ) -> Self {
        Self {
            screen_width,
            screen_height,
            ..self
        }
    }

    pub(crate) const fn with_device_pixel_ratio(self, device_pixel_ratio: Option<f64>) -> Self {
        Self {
            device_pixel_ratio,
            ..self
        }
    }

    pub(crate) fn from_viewport_surface(surface: crate::protocol_types::ViewportSurface) -> Self {
        let (width, height) = surface.css_layout_size();
        Self::new(Some(f64::from(width)), Some(f64::from(height)))
            .with_screen_size(
                Some(f64::from(surface.screen_width)),
                Some(f64::from(surface.screen_height)),
            )
            .with_device_pixel_ratio(Some(surface.device_pixel_ratio))
    }
}

impl From<Option<f64>> for StyleViewport {
    fn from(width: Option<f64>) -> Self {
        Self::from_width(width)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct StyloStyleEnvironment {
    media_type: StyloStyleMediaType,
    color_scheme: StyloStyleColorScheme,
    page_color_scheme_bits: u8,
    reduced_motion: StyloStyleReducedPreference,
    reduced_data: StyloStyleReducedPreference,
    reduced_transparency: StyloStyleReducedPreference,
    contrast: StyloStyleContrastPreference,
    forced_colors: StyloStyleForcedColors,
    preferred_text_scale_percent: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleMediaType {
    #[default]
    Screen,
    Print,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleColorScheme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleReducedPreference {
    #[default]
    NoPreference,
    Reduce,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleContrastPreference {
    More,
    Less,
    Custom,
    #[default]
    NoPreference,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleForcedColors {
    #[default]
    None,
    Active,
}

impl StyloStyleEnvironment {
    pub(crate) fn from_emulated_media(
        overrides: &crate::protocol_types::EmulatedMediaOverrides,
    ) -> Self {
        Self {
            media_type: if overrides.media.as_deref() == Some("print") {
                StyloStyleMediaType::Print
            } else {
                StyloStyleMediaType::Screen
            },
            color_scheme: if overrides.color_scheme.as_deref() == Some("dark") {
                StyloStyleColorScheme::Dark
            } else {
                StyloStyleColorScheme::Light
            },
            page_color_scheme_bits: 0,
            reduced_motion: match overrides.reduced_motion.as_deref() {
                Some("reduce") => StyloStyleReducedPreference::Reduce,
                Some("no-preference") | None => StyloStyleReducedPreference::NoPreference,
                Some(_) => StyloStyleReducedPreference::NoPreference,
            },
            reduced_data: StyloStyleReducedPreference::NoPreference,
            reduced_transparency: match overrides.reduced_transparency.as_deref() {
                Some("reduce") => StyloStyleReducedPreference::Reduce,
                Some("no-preference") | None => StyloStyleReducedPreference::NoPreference,
                Some(_) => StyloStyleReducedPreference::NoPreference,
            },
            contrast: match overrides.contrast.as_deref() {
                Some("more") => StyloStyleContrastPreference::More,
                Some("less") => StyloStyleContrastPreference::Less,
                Some("custom") => StyloStyleContrastPreference::Custom,
                Some("no-preference") | None => StyloStyleContrastPreference::NoPreference,
                Some(_) => StyloStyleContrastPreference::NoPreference,
            },
            forced_colors: match overrides.forced_colors.as_deref() {
                Some("active") => StyloStyleForcedColors::Active,
                Some("none") | None => StyloStyleForcedColors::None,
                Some(_) => StyloStyleForcedColors::None,
            },
            preferred_text_scale_percent: overrides.os_text_scale_percent,
        }
    }

    pub(crate) fn for_document(mut self, host: &DomHost, document: Option<DomHandle>) -> Self {
        if self.preferred_text_scale_percent.is_some()
            && !document.is_some_and(|document| document_uses_text_scale(host, document))
        {
            self.preferred_text_scale_percent = None;
        }
        self.with_page_color_schemes(
            document
                .map(|document| {
                    crate::document_color_scheme::document_page_color_schemes(host, document)
                })
                .unwrap_or_default(),
        )
    }

    pub(super) fn preferred_text_scale(self) -> f32 {
        f32::from(self.preferred_text_scale_percent.unwrap_or(100)) / 100.0
    }

    pub(crate) fn with_page_color_schemes(mut self, color_schemes: ColorSchemeFlags) -> Self {
        self.page_color_scheme_bits = color_schemes.bits();
        self
    }

    pub(super) fn stylo_media_type(self) -> MediaType {
        match self.media_type {
            StyloStyleMediaType::Screen => MediaType::screen(),
            StyloStyleMediaType::Print => MediaType::print(),
        }
    }

    pub(super) fn stylo_prefers_color_scheme(self) -> PrefersColorScheme {
        match self.color_scheme {
            StyloStyleColorScheme::Light => PrefersColorScheme::Light,
            StyloStyleColorScheme::Dark => PrefersColorScheme::Dark,
        }
    }

    pub(super) fn stylo_page_color_schemes(self) -> ColorSchemeFlags {
        ColorSchemeFlags::from_bits_retain(self.page_color_scheme_bits)
    }

    pub(super) fn stylo_media_feature_preferences(self) -> ServoMediaFeaturePreferences {
        ServoMediaFeaturePreferences {
            prefers_reduced_motion: self.reduced_motion.prefers_reduced(),
            prefers_reduced_data: self.reduced_data.prefers_reduced(),
            prefers_reduced_transparency: self.reduced_transparency.prefers_reduced(),
            prefers_contrast: match self.contrast {
                StyloStyleContrastPreference::More => PrefersContrast::More,
                StyloStyleContrastPreference::Less => PrefersContrast::Less,
                StyloStyleContrastPreference::Custom => PrefersContrast::Custom,
                StyloStyleContrastPreference::NoPreference => PrefersContrast::NoPreference,
            },
            forced_colors: match self.forced_colors {
                StyloStyleForcedColors::None => ForcedColors::None,
                StyloStyleForcedColors::Active => ForcedColors::Active,
            },
        }
    }
}

fn document_uses_text_scale(host: &DomHost, document: DomHandle) -> bool {
    host.html_elements_by_local_name_in_document_tree_order(document, "meta")
        .into_iter()
        .filter_map(|handle| host.node(handle).and_then(Node::as_element))
        .find(|meta| {
            meta.attribute_ns("", "name")
                .is_some_and(|name| name.eq_ignore_ascii_case("text-scale"))
        })
        .and_then(|meta| meta.attribute_ns("", "content"))
        .is_some_and(|content| content.eq_ignore_ascii_case("scale"))
}

impl StyloStyleReducedPreference {
    fn prefers_reduced(self) -> bool {
        matches!(self, Self::Reduce)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StyleWorldEnvironment {
    pub(super) viewport: StyleViewport,
    pub(super) media: StyloStyleEnvironment,
    pub(super) quirks_mode: style::context::QuirksMode,
    pub(super) tree_scope_versions: StyleTreeScopeVersions,
}

impl StyleWorldEnvironment {
    pub(crate) fn new(
        viewport: StyleViewport,
        media: StyloStyleEnvironment,
        quirks_mode: style::context::QuirksMode,
        tree_scope_versions: StyleTreeScopeVersions,
    ) -> Self {
        Self {
            viewport,
            media,
            quirks_mode,
            tree_scope_versions,
        }
    }
}
